use clap::Args;
use parking_lot::RwLock;
use sqlx::{
    postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode},
    query, query_file, query_file_as, ConnectOptions,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tracing::{error, info, instrument, log::LevelFilter, warn};

const APPLICATION_NAME: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Args)]
pub struct Options {
    /// The default database to connect to
    #[arg(
        long = "database-default-dbname",
        default_value = "postgres",
        env = "DATABASE_DEFAULT_DBNAME"
    )]
    pub default_dbname: String,

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
    default_dbname: String,
    pools: Arc<RwLock<HashMap<String, PgPool>>>,
}

impl Databases {
    pub async fn new(opts: &Options) -> Result<Self> {
        // Construct the connection options
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

        let databases = Databases {
            options: Arc::new(options),
            pools: Arc::new(RwLock::new(HashMap::new())),
            default_dbname: opts.default_dbname.clone(),
        };
        databases.ensure_configuration(&opts.username).await?;

        Ok(databases)
    }

    /// Ensure the pgbouncer user is setup and the connecting user has the correct permissions
    #[instrument(skip(self))]
    async fn ensure_configuration(&self, connecting_user: &str) -> Result<()> {
        let default = self.get(&self.default_dbname).await?;

        // Ensure the connecting user has the correct permissions
        let connecting_user = query_file_as!(User, "queries/user-permissions.sql", connecting_user)
            .fetch_one(&default.pool)
            .await?;
        if !(connecting_user.create_role && connecting_user.create_db) {
            error!(
                "user {:?} must have create role and create db permissions",
                connecting_user.username
            );
            return Err(Error::InvalidPermissions);
        }
        info!("current user has required permissions");

        // Ensure the pgbouncer user exists
        let pgbouncer = query_file_as!(User, "queries/user-permissions.sql", "pgbouncer")
            .fetch_optional(&default.pool)
            .await?;

        match pgbouncer {
            Some(user) => {
                info!(
                    %user.can_login,
                    %user.create_db,
                    %user.create_role,
                    %user.bypass_rls,
                    %user.superuser,
                    "pgbouncer user already exists"
                );
                if !user.can_login {
                    warn!("pgbouncer user should be able to login");
                }
            }
            None => {
                warn!("pgbouncer user does not exist, creating...");
                query!("CREATE USER pgbouncer WITH LOGIN NOSUPERUSER NOCREATEROLE NOCREATEDB NOREPLICATION NOBYPASSRLS").execute(&default.pool).await?;
            }
        }

        Ok(())
    }

    /// Fetch a connection pool for the specified database
    #[instrument(skip(self))]
    pub async fn get(&self, database: &str) -> Result<Database> {
        {
            let pools = self.pools.read();
            if let Some(pool) = pools.get(database) {
                return Ok(Database { pool: pool.clone() });
            }
        }

        let pool = self.open(database).await?;
        let stored = pool.clone();

        {
            let mut pools = self.pools.write();
            pools.insert(database.to_string(), stored);
        }

        Ok(Database { pool })
    }

    /// Open a new connection to the database
    async fn open(&self, database: &str) -> Result<PgPool> {
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

#[derive(Debug)]
struct User {
    username: String,
    can_login: bool,
    create_role: bool,
    create_db: bool,
    bypass_rls: bool,
    superuser: bool,
}

fn non_empty_optional(s: &String) -> Option<&String> {
    match s.is_empty() {
        true => None,
        false => Some(s),
    }
}

/// A convince wrapper around a connection pool
#[derive(Clone, Debug)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Ensure the pgbouncer schema exists and has the proper permissions
    #[instrument(skip_all)]
    pub async fn ensure_schema(&self) -> Result<()> {
        query!("CREATE SCHEMA IF NOT EXISTS pgbouncer")
            .execute(&self.pool)
            .await?;

        query!("GRANT USAGE ON SCHEMA pgbouncer TO pgbouncer")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Ensure the authentication function exists and has the proper permissions
    #[instrument(skip_all)]
    pub async fn ensure_authentication_query(&self) -> Result<()> {
        let schema = query!("SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = 'pgbouncer'")
            .fetch_one(&self.pool)
            .await?;
        let user_lookup =
            query!("SELECT oid FROM pg_catalog.pg_proc WHERE proname = 'user_lookup' AND pronamespace = $1", schema.oid)
                .fetch_optional(&self.pool)
                .await?;

        if user_lookup.is_none() {
            query_file!("queries/authentication-query-function.sql")
                .execute(&self.pool)
                .await?;
        }

        query!("REVOKE ALL ON FUNCTION pgbouncer.user_lookup(text) FROM public, pgbouncer")
            .execute(&self.pool)
            .await?;
        query!("GRANT EXECUTE ON FUNCTION pgbouncer.user_lookup(text) TO pgbouncer")
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid permissions for connected user")]
    InvalidPermissions,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}