use clap::Args;
use eyre::WrapErr;
use sqlx::{
    postgres::{PgConnectOptions, PgSslMode},
    ConnectOptions, PgPool,
};
use std::{net::SocketAddr, path::PathBuf};
use tracing::{info, instrument, log::LevelFilter};

const APPLICATION_NAME: &'static str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// Launch the server
pub async fn launch(args: ServerArgs) -> eyre::Result<()> {
    let _db = args.connect().await?;

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
    pub async fn connect(&self) -> eyre::Result<PgPool> {
        let mut options = PgConnectOptions::new()
            .application_name(APPLICATION_NAME)
            .port(self.database.port)
            .username(&self.database.username)
            .ssl_mode(self.database.ssl_mode);
        options.log_statements(LevelFilter::Debug);

        if let Some(password) = non_empty_optional_string(self.database.password.as_ref()) {
            options = options.password(&password);
        }

        if let Some(host) = non_empty_optional_string(self.database.host.as_ref()) {
            options = options.host(&host);
        } else {
            options = options.socket(&self.database.socket);
        }

        let db = PgPool::connect_with(options)
            .await
            .wrap_err("failed to connect to database")?;
        info!("database connected");

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

fn non_empty_optional_string(str: Option<&String>) -> Option<&String> {
    str.map(|s| if s.is_empty() { None } else { Some(s) })
        .flatten()
}
