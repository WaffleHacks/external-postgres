use axum::Server;
use clap::Args;
use eyre::WrapErr;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::{signal, task::JoinHandle};
use tracing::{error, info};

mod controller;
mod database;
mod http;
mod kube;

use self::kube::Kube;
use controller::Controller;
use database::Databases;

/// Launch the server
pub async fn launch(args: ServerArgs) -> eyre::Result<()> {
    let databases = Databases::new(&args.database)
        .await
        .wrap_err("failed to connect to database")?;
    let (controller, handle) = Controller::start(databases.clone());
    let kube = Kube::new(args.kubeconfig, args.kube_context, controller.clone());

    // Launch the server
    info!(address = %args.management_address, "listening and ready to handle requests");
    Server::bind(&args.management_address)
        .serve(http::router(controller.clone(), databases, kube.clone()).into_make_service())
        .with_graceful_shutdown(shutdown(controller, handle, kube))
        .await
        .wrap_err("failed to start server")?;

    Ok(())
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[command(flatten)]
    database: database::Options,

    /// The address for the management server to listen on
    #[arg(
        short,
        long,
        default_value = "127.0.0.1:8032",
        env = "MANAGEMENT_ADDRESS"
    )]
    pub management_address: SocketAddr,

    /// The path to the kubeconfig file
    #[arg(short, long, default_value = "~/.kube/config", env = "KUBECONFIG")]
    pub kubeconfig: PathBuf,

    /// The Kubernetes context to use
    #[arg(short = 'c', long, env = "KUBE_CONTEXT")]
    pub kube_context: Option<String>,
}

/// Wait for signals for terminating
async fn shutdown(controller: Controller, handle: JoinHandle<()>, kube: Kube) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install ctrl+c handler")
    };
    let terminate = async {
        use signal::unix::SignalKind;

        signal::unix::signal(SignalKind::terminate())
            .expect("failed to install sigterm handler")
            .recv()
            .await
    };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    kube.stop().await;

    controller.stop();
    if let Err(error) = handle.await {
        error!(%error, "failed to stop controller");
    }

    info!("server successfully shutdown");
    info!("goodbye! :)");
}
