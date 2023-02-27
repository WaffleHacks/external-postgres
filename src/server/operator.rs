use super::database::{self, Databases};
use futures::StreamExt;
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
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle, time};
use tracing::{debug, error, info, instrument};

#[derive(Clone, Debug)]
pub struct Operator(Arc<KubeInner>);

#[derive(Debug)]
struct KubeInner {
    databases: Databases,
    kubeconfig: PathBuf,
    kube_context: Option<String>,
    handle: Mutex<Option<KubeControllerHandle>>,
}

#[derive(Debug)]
struct KubeControllerHandle {
    stop: oneshot::Sender<()>,
    join: JoinHandle<()>,
}

impl Operator {
    /// Create a new kubernetes operator
    pub fn new(kubeconfig: PathBuf, kube_context: Option<String>, databases: Databases) -> Self {
        let kubeconfig = shellexpand::tilde(&kubeconfig.as_os_str().to_string_lossy())
            .to_string()
            .into();

        let operator = Operator(Arc::new(KubeInner {
            databases,
            kubeconfig,
            kube_context,
            handle: Mutex::default(),
        }));

        // Launch the controller if the kubeconfig exists
        if operator.0.kubeconfig.exists() {
            operator.spawn();
        } else {
            tokio::spawn(operator.clone().wait_for_kubeconfig());
        }

        operator
    }

    /// Stop the operator
    pub async fn stop(self) {
        let handle = {
            let mut handle = self.0.handle.lock();
            handle.take()
        };

        if let Some(handle) = handle {
            handle.stop.send(()).unwrap();

            if let Err(error) = handle.join.await {
                error!(%error, "failed to stop kube controller");
            }
        }
    }

    /// Wait for the kubeconfig at the specified path to exist
    #[instrument(skip_all, fields(path = %self.0.kubeconfig.display()))]
    async fn wait_for_kubeconfig(self) {
        loop {
            if self.0.kubeconfig.exists() {
                info!("kube config exists, launching controller");
                self.spawn();
            }

            debug!("kubeconfig not found, waiting...");
            time::sleep(Duration::from_secs(5)).await;
        }
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
                    let databases = self.0.databases.clone();
                    async move {
                        finalizer(
                            &databases_api,
                            "external-postgres.wafflehacks.cloud/cleanup",
                            database,
                            |event| async {
                                match event {
                                    Event::Apply(object) => apply(object, databases).await,
                                    Event::Cleanup(object) => cleanup(object, databases).await,
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
async fn apply(object: Arc<Database>, databases: Databases) -> Result<Action> {
    let name = name_for_database(&object)?;

    databases.ensure(&name, &object.spec.password).await?;

    // TODO: expose connection details across namespaces

    Ok(Action::await_change())
}

/// Cleanup databases from the CRD
#[instrument(skip_all)]
async fn cleanup(object: Arc<Database>, databases: Databases) -> Result<Action> {
    let name = name_for_database(&object)?;
    databases
        .remove(&name, object.spec.retain_on_delete)
        .await?;

    Ok(Action::await_change())
}

fn name_for_database(database: &Database) -> Result<String> {
    database.metadata.name.clone().ok_or(Error::NoName)
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
    password: String,
    /// Whether to retain the database's data on deletion
    #[serde(default)]
    retain_on_delete: bool,
    /// Specification for the connection secret
    secret: DatabaseSecret,
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
    #[error(transparent)]
    Database(#[from] database::Error),
    #[error(transparent)]
    Kubernetes(#[from] kube::Error),
    #[error(transparent)]
    Wait(#[from] wait::Error),
}
