use axum::Server;
use clap::Args;
use eyre::WrapErr;
use std::net::SocketAddr;
use tokio::signal;
use tracing::info;

mod database;
mod http;

use database::Databases;

/// Launch the server
pub async fn launch(args: ServerArgs) -> eyre::Result<()> {
    let databases = Databases::new(&args.database)
        .await
        .wrap_err("failed to connect to database")?;

    // Launch the server
    info!(address = %args.management_address, "listening and ready to handle requests");
    Server::bind(&args.management_address)
        .serve(http::router(databases).into_make_service())
        .with_graceful_shutdown(shutdown())
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
}

/// Wait for signals for terminating
async fn shutdown() {
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

    info!("server successfully shutdown");
    info!("goodbye! :)");
}
