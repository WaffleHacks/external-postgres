use super::database::{self, Databases};
use clap::Args;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{Patch, PatchParams},
    client::Client,
    config::{Config, KubeConfigOptions, Kubeconfig},
    runtime::{
        controller::Action,
        finalizer::{finalizer, Event},
        wait::{self, await_condition, conditions},
        Controller,
    },
    Api, CustomResource, CustomResourceExt, ResourceExt,
};
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgSslMode;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};
use tracing::{debug, error, info, instrument, warn};

#[derive(Clone, Debug, Args)]
pub struct ConnectionInfo {
    /// The host for clients within the cluster to connect with
    #[arg(
        long = "kube-database-host",
        default_value = "postgres",
        env = "KUBE_DATABASE_HOST"
    )]
    pub remote_host: String,

    /// The port of the server to connect to
    #[arg(
        long = "kube-database-port",
        default_value_t = 5432,
        env = "KUBE_DATABASE_PORT"
    )]
    pub remote_port: u16,

    /// The SSL connection mode to use
    #[arg(
        long = "kube-database-ssl-mode",
        default_value = "prefer",
        env = "KUBE_DATABASE_SSL_MODE"
    )]
    pub sslmode: PgSslMode,
}

impl ConnectionInfo {
    fn into_secret_data(self) -> BTreeMap<String, String> {
        let mut data = BTreeMap::new();

        data.insert(String::from("PGHOST"), self.remote_host);
        data.insert(String::from("PGPORT"), format!("{}", self.remote_port));
        data.insert(
            String::from("PGSSLMODE"),
            format!("{:?}", self.sslmode).to_lowercase(),
        );

        data
    }
}

#[derive(Clone, Debug)]
pub struct Operator(Arc<KubeInner>);

#[derive(Debug)]
struct KubeInner {
    databases: Databases,
    kubeconfig: PathBuf,
    kube_context: Option<String>,
    handle: Mutex<Option<KubeControllerHandle>>,
    secret_data: BTreeMap<String, String>,
}

#[derive(Debug)]
struct KubeControllerHandle {
    stop: oneshot::Sender<()>,
    join: JoinHandle<()>,
}

impl Operator {
    /// Create a new kubernetes operator
    #[instrument(name = "operator", skip(connection_info, databases))]
    pub fn new(
        kubeconfig: PathBuf,
        kube_context: Option<String>,
        connection_info: ConnectionInfo,
        databases: Databases,
    ) -> Self {
        let kubeconfig = shellexpand::tilde(&kubeconfig.as_os_str().to_string_lossy())
            .to_string()
            .into();

        let operator = Operator(Arc::new(KubeInner {
            databases,
            kubeconfig,
            kube_context,
            handle: Mutex::default(),
            secret_data: connection_info.into_secret_data(),
        }));

        // Launch the controller if the kubeconfig exists
        if operator.0.kubeconfig.exists() {
            info!("kubeconfig exists, launching...");
            operator.spawn();
        } else {
            warn!(path = %operator.0.kubeconfig.display(), "could not find kubeconfig");
            info!("run `external-postgres operator enable` once the kubeconfig exists");
        }

        operator
    }

    /// Try to spawn the operator
    #[instrument(skip_all, fields(path = %self.0.kubeconfig.display()))]
    pub fn start(&self) -> bool {
        {
            let handle = self.0.handle.lock();
            if handle.is_some() {
                return true;
            }
        }

        let exists = self.0.kubeconfig.exists();
        if exists {
            info!("kubeconfig exists, launching controller");
            self.spawn();
        }

        exists
    }

    /// Stop the operator
    #[instrument(skip_all)]
    pub async fn stop(&self) -> bool {
        let handle = {
            let mut handle = self.0.handle.lock();
            handle.take()
        };

        if let Some(handle) = handle {
            handle.stop.send(()).unwrap();

            if let Err(error) = handle.join.await {
                // Simply log the error, as there's nothing we can do about it
                error!(%error, "failed to stop kube controller");
            }

            true
        } else {
            false
        }
    }

    /// Check whether the operator is running
    pub fn status(&self) -> bool {
        let handle = self.0.handle.lock();
        handle.is_some()
    }

    /// Launch the operator in a separate task
    fn spawn(&self) {
        let (tx, rx) = oneshot::channel();
        let mut handle = self.0.handle.lock();
        *handle = Some(KubeControllerHandle {
            stop: tx,
            join: tokio::spawn(self.clone().operator(rx)),
        });
    }

    /// Runs the kubernetes operator
    async fn operator(self, stop: oneshot::Receiver<()>) {
        let kubeconfig = Kubeconfig::read_from(&self.0.kubeconfig).unwrap();
        let config = Config::from_custom_kubeconfig(
            kubeconfig,
            &KubeConfigOptions {
                context: self.0.kube_context.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let client = Client::try_from(config).unwrap();
        if let Err(error) = apply_crd(client.clone()).await {
            error!(%error, "failed to apply CRD");
        }

        let databases = Api::<Database>::all(client.clone());
        Controller::new(databases, Default::default())
            .graceful_shutdown_on(async {
                stop.await.unwrap();
                debug!("shutdown signal received");
            })
            .run(
                |database, _| {
                    let databases_api = Api::<Database>::all(client.clone());
                    let client = client.clone();

                    let base_secret_data = self.0.secret_data.clone();
                    let databases = self.0.databases.clone();

                    async move {
                        finalizer(
                            &databases_api,
                            "external-postgres.wafflehacks.cloud/cleanup",
                            database,
                            |event| async {
                                match event {
                                    Event::Apply(object) => {
                                        apply(object, databases, base_secret_data, client).await
                                    }
                                    Event::Cleanup(object) => {
                                        cleanup(object, databases, client).await
                                    }
                                }
                            },
                        )
                        .await
                    }
                },
                |object, error, _| {
                    use std::error::Error;

                    let source = error.source().map(ToString::to_string).unwrap_or_default();
                    error!(r#for = object.name_any(), %error, %source, "failed to reconcile");
                    Action::requeue(Duration::from_secs(5))
                },
                Arc::new(()),
            )
            .for_each(|_| futures::future::ready(()))
            .await;
    }
}

/// Apply changes from the CRD
#[instrument(skip_all)]
async fn apply(
    object: Arc<Database>,
    databases: Databases,
    mut secret_data: BTreeMap<String, String>,
    client: Client,
) -> Result<Action> {
    let name = name_for_database(&object)?;
    let password = password_from_spec(&object, client.clone()).await?;

    databases.ensure(&name, &password).await?;
    info!("ensured database exists");

    // Populate the secret data
    secret_data.insert(String::from("PGUSER"), name.clone());
    secret_data.insert(String::from("PGPASSWORD"), password.clone());
    secret_data.insert(String::from("PGDATABASE"), name.clone());

    secret_data.insert(
        String::from("DATABASE_URL"),
        format!(
            "postgresql://{}:{}@{}:{}/{}?sslmode={}",
            name,
            password,
            secret_data.get("PGHOST").unwrap(),
            secret_data.get("PGPORT").unwrap(),
            name,
            secret_data.get("PGSSLMODE").unwrap()
        ),
    );

    let secret_name = secret_name_for_database(&object);
    for namespace in &object.spec.secret.namespaces {
        let secrets = Api::<Secret>::namespaced(client.clone(), namespace);
        secrets
            .patch(
                &secret_name,
                &PatchParams::apply("external-postgres.wafflehacks.cloud").force(),
                &Patch::Apply(&Secret {
                    metadata: ObjectMeta {
                        name: secret_name.clone().into(),
                        ..Default::default()
                    },
                    string_data: secret_data.clone().into(),
                    ..Default::default()
                }),
            )
            .await?;

        info!(%namespace, "added secret to namespace");
    }

    Ok(Action::await_change())
}

/// Retrieve the password from the database spec
#[instrument(skip_all)]
async fn password_from_spec(object: &Database, client: Client) -> Result<String> {
    match &object.spec.password {
        DatabasePassword::Value(v) => Ok(v.clone()),
        DatabasePassword::FromSecret(spec) => {
            let secrets = Api::<Secret>::namespaced(client, &spec.namespace);
            let secret = secrets.get(&spec.name).await.map_err(|e| match e {
                kube::Error::Api(response) if response.code == 404 => Error::NoPassword,
                e => Error::from(e),
            })?;
            info!(namespace = %spec.namespace, name = %spec.name, "found secret");

            let password_bytes = secret
                .data
                .unwrap_or_default()
                .remove(&spec.key)
                .ok_or(Error::NoPassword)?;
            info!(key = %spec.key, "found key in secret");
            let password =
                String::from_utf8(password_bytes.0).map_err(|_| Error::InvalidPassword)?;
            debug!(%password);

            Ok(password)
        }
    }
}

/// Cleanup databases from the CRD
#[instrument(skip_all)]
async fn cleanup(object: Arc<Database>, databases: Databases, client: Client) -> Result<Action> {
    let name = name_for_database(&object)?;
    databases
        .remove(&name, object.spec.retain_on_delete)
        .await?;

    let secret_name = secret_name_for_database(&object);
    for namespace in &object.spec.secret.namespaces {
        let secrets = Api::<Secret>::namespaced(client.clone(), namespace);
        secrets.delete(&secret_name, &Default::default()).await?;

        info!(%namespace, "removed secret from namespace");
    }

    Ok(Action::await_change())
}

fn name_for_database(database: &Database) -> Result<String> {
    database.metadata.name.clone().ok_or(Error::NoName)
}

fn secret_name_for_database(database: &Database) -> String {
    let name = name_for_database(database).unwrap();
    database
        .spec
        .secret
        .name
        .clone()
        .unwrap_or_else(|| format!("database-{name}-secret"))
}

#[instrument(skip_all)]
async fn apply_crd(client: Client) -> Result<()> {
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;

    let api = Api::<CustomResourceDefinition>::all(client);

    let crd = Database::crd();
    let name = crd.metadata.name.as_ref().unwrap();

    let params = PatchParams::apply("external-postgres.wafflehacks.cloud").force();
    api.patch(name, &params, &Patch::Apply(&crd)).await?;
    await_condition(api, name, conditions::is_crd_established()).await?;

    info!("CRD successfully applied");

    Ok(())
}

#[derive(Clone, CustomResource, Debug, Deserialize, JsonSchema, Serialize)]
#[kube(
    group = "external-postgres.wafflehacks.cloud",
    version = "v1",
    kind = "Database",
    singular = "database",
    plural = "databases",
    shortname = "db",
    shortname = "dbs"
)]
#[serde(rename_all = "camelCase")]
struct DatabaseSpec {
    /// The password for the database
    #[validate(length(min = 1))]
    password: DatabasePassword,
    /// Whether to retain the database's data on deletion
    #[serde(default)]
    retain_on_delete: bool,
    /// Specification for the connection secret
    secret: DatabaseSecret,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
enum DatabasePassword {
    Value(#[validate(length(min = 1))] String),
    FromSecret(DatabasePasswordSecret),
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabasePasswordSecret {
    /// The name of the secret to pull from
    name: String,
    /// The key to retrieve the password from
    key: String,
    /// The namespace the secret resides in
    namespace: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseSecret {
    /// The custom name for the secret, defaults to database-<dbname>-secret
    #[validate(length(min = 1))]
    name: Option<String>,
    /// The namespaces to replicate the secret to
    #[serde(default)]
    namespaces: Vec<String>,
}

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("resource does not have a name")]
    NoName,
    #[error("could not find the password")]
    NoPassword,
    #[error("invalid password sequence, likely invalid utf-8")]
    InvalidPassword,
    #[error(transparent)]
    Database(#[from] database::Error),
    #[error(transparent)]
    Kubernetes(#[from] kube::Error),
    #[error(transparent)]
    Wait(#[from] wait::Error),
}
