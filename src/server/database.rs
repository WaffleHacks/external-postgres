use clap::Args;
use parking_lot::RwLock;
use sqlx::{
    postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode},
    ConnectOptions,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tracing::{info, instrument, log::LevelFilter};

const APPLICATION_NAME: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Args)]
pub struct Options {
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

/// Manage the connection pools of different databases on the specified server
#[derive(Clone, Debug)]
pub struct Databases {
    options: Arc<PgConnectOptions>,
    pools: Arc<RwLock<HashMap<String, PgPool>>>,
}

impl Databases {
    /// Fetch a connection pool for the specified database
    #[instrument(skip(self))]
    pub async fn get(&self, database: &str) -> sqlx::Result<PgPool> {
        {
            let pools = self.pools.read();
            if let Some(pool) = pools.get(database) {
                return Ok(pool.clone());
            }
        }

        let pool = self.open(database).await?;
        let stored = pool.clone();

        {
            let mut pools = self.pools.write();
            pools.insert(database.to_string(), stored);
        }

        Ok(pool)
    }

    /// Open a new connection to the database
    async fn open(&self, database: &str) -> sqlx::Result<PgPool> {
        let options = self.options.as_ref().clone().database(database);

        // Create a pool with a single short-lived connection as we will
        // 1. only be performing actions one-at-a-time
        // 2. infrequently using connections
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .idle_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        info!("connected to database");

        Ok(pool)
    }

    /// Release connection pool for the specified database
    pub async fn release(&self, database: &str) {
        let pool = {
            let mut pools = self.pools.write();
            pools.remove(database)
        };

        if let Some(pool) = pool {
            pool.close().await;
        }
    }
}

impl From<&Options> for Databases {
    fn from(opts: &Options) -> Self {
        let mut options = PgConnectOptions::new()
            .application_name(APPLICATION_NAME)
            .port(opts.port)
            .username(&opts.username)
            .ssl_mode(opts.ssl_mode);
        options.log_statements(LevelFilter::Debug);

        if let Some(password) = opts.password.as_ref().and_then(non_empty_optional) {
            options = options.password(password);
        }

        if let Some(host) = opts.host.as_ref().and_then(non_empty_optional) {
            options = options.host(host);
        } else {
            options = options.socket(&opts.socket);
        }

        Databases {
            options: Arc::new(options),
            pools: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

fn non_empty_optional(s: &String) -> Option<&String> {
    match s.is_empty() {
        true => None,
        false => Some(s),
    }
}
