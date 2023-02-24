use axum::Server;
use clap::Args;
use eyre::WrapErr;
use sqlx::{
    postgres::{PgConnectOptions, PgSslMode},
    ConnectOptions, Connection, PgConnection,
};
use std::{net::SocketAddr, path::PathBuf};
use tokio::signal;
use tracing::{info, instrument, log::LevelFilter};

mod http;

const APPLICATION_NAME: &'static str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// Launch the server
pub async fn launch(args: ServerArgs) -> eyre::Result<()> {
    let _db = args.connect("postgres").await?;

    // Launch the server
    info!(address = %args.management_address, "listening and ready to handle requests");
    Server::bind(&args.management_address)
        .serve(http::router().into_make_service())
        .with_graceful_shutdown(shutdown())
        .await
        .wrap_err("failed to start server")?;

    Ok(())
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[command(flatten)]
    database: DatabaseOptions,

    /// The address for the management server to listen on
    #[arg(
        short,
        long,
        default_value = "127.0.0.1:8032",
        env = "MANAGEMENT_ADDRESS"
    )]
    pub management_address: SocketAddr,
}

impl ServerArgs {
    /// Connect to the database
    #[instrument(skip_all)]
    pub async fn connect(&self, database: &str) -> eyre::Result<PgConnection> {
        let mut options = PgConnectOptions::new()
            .application_name(APPLICATION_NAME)
            .port(self.database.port)
            .username(&self.database.username)
            .ssl_mode(self.database.ssl_mode)
            .database(database);
        options.log_statements(LevelFilter::Debug);

        if let Some(password) = non_empty_optional_string(self.database.password.as_ref()) {
            options = options.password(&password);
        }

        if let Some(host) = non_empty_optional_string(self.database.host.as_ref()) {
            options = options.host(&host);
        } else {
            options = options.socket(&self.database.socket);
        }

        let mut db = PgConnection::connect_with(&options)
            .await
            .wrap_err("failed to connect to database")?;
        info!("database connection opened");

        db.ping().await.wrap_err("failed to ping database")?;
        info!("connection works!");

        Ok(db)
    }
}

#[derive(Debug, Args)]
struct DatabaseOptions {
    /// The path to the socket directory
    #[arg(long = "database-socket-dir", env = "DATABASE_SOCKET_DIR")]
    pub socket: PathBuf,

    /// The host to connect to
    #[arg(long = "database-host", env = "DATABASE_HOST")]
    pub host: Option<String>,

    /// The port of the server to connect to
    #[arg(long = "database-port", env = "DATABASE_PORT")]
    pub port: u16,

    /// The database user to connect as
    #[arg(long = "database-username", env = "DATABASE_USERNAME")]
    pub username: String,

    /// The database password to connect with
    #[arg(long = "database-password", env = "DATABASE_PASSWORD")]
    pub password: Option<String>,

    /// The SSL connection mode to use
    #[arg(
        long = "database-ssl-mode",
        default_value = "prefer",
        env = "DATABASE_SSL_MODE"
    )]
    pub ssl_mode: PgSslMode,
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

fn non_empty_optional_string(str: Option<&String>) -> Option<&String> {
    str.map(|s| if s.is_empty() { None } else { Some(s) })
        .flatten()
}
