use super::controller::Controller;
use futures::StreamExt;
use kube::{
    api::{Patch, PatchParams},
    client::Client,
    config::{Config, KubeConfigOptions, Kubeconfig},
    runtime::{
        controller::Action,
        finalizer::{finalizer, Event},
        wait::{self, await_condition, conditions},
        Controller as Operator,
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
pub struct Kube(Arc<KubeInner>);

#[derive(Debug)]
struct KubeInner {
    controller: Controller,
    kubeconfig: PathBuf,
    kube_context: Option<String>,
    handle: Mutex<Option<KubeControllerHandle>>,
}

#[derive(Debug)]
struct KubeControllerHandle {
    stop: oneshot::Sender<()>,
    join: JoinHandle<()>,
}

impl Kube {
    /// Create a new kube watcher
    pub fn new(kubeconfig: PathBuf, kube_context: Option<String>, controller: Controller) -> Self {
        let kubeconfig = shellexpand::tilde(&kubeconfig.as_os_str().to_string_lossy())
            .to_string()
            .into();

        let kube = Kube(Arc::new(KubeInner {
            controller,
            kubeconfig,
            kube_context,
            handle: Mutex::default(),
        }));

        // Launch the controller if the kubeconfig exists
        if kube.0.kubeconfig.exists() {
            kube.launch_operator();
        } else {
            tokio::spawn(kube.clone().wait_for_kubeconfig());
        }

        kube
    }

    /// Stop the kube watcher
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
                self.launch_operator();
            }

            debug!("kubeconfig not found, waiting...");
            time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// Launch the operator in a separate task
    fn launch_operator(&self) {
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
        Operator::new(databases, Default::default())
            .graceful_shutdown_on(async {
                stop.await.unwrap();
                debug!("shutdown signal received");
            })
            .run(
                |database, _| {
                    let databases = Api::<Database>::all(client.clone());
                    let controller = self.0.controller.clone();
                    async move {
                        finalizer(
                            &databases,
                            "external-postgres.wafflehacks.cloud/cleanup",
                            database,
                            |event| async {
                                match event {
                                    Event::Apply(database) => apply(database, controller).await,
                                    Event::Cleanup(database) => cleanup(database, controller).await,
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
async fn apply(database: Arc<Database>, controller: Controller) -> Result<Action> {
    let name = name_for_database(&database)?;
    if let Some(_password) = controller.create(name).await {
        // TODO: expose password to k8s services
    }

    Ok(Action::await_change())
}

/// Cleanup databases from the CRD
#[instrument(skip_all)]
async fn cleanup(database: Arc<Database>, controller: Controller) -> Result<Action> {
    let name = name_for_database(&database)?;
    controller.remove(name, database.spec.retain_on_delete);

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
    Kubernetes(#[from] kube::Error),
    #[error(transparent)]
    Wait(#[from] wait::Error),
}
