use crate::constants::APPLICATION_NAME;
use clap::Args;
use parking_lot::RwLock;
use sqlx::{
    postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode},
    query, query_file, query_file_as, ConnectOptions,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tracing::{debug, error, info, instrument, log::LevelFilter, warn};

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
pub struct Databases(Arc<DatabasesInner>);

#[derive(Debug)]
struct DatabasesInner {
    options: PgConnectOptions,
    pools: RwLock<HashMap<String, PgPool>>,

    default_dbname: String,
    default_username: String,
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

        let databases = Databases(Arc::new(DatabasesInner {
            options,
            pools: RwLock::new(HashMap::new()),
            default_dbname: opts.default_dbname.clone(),
            default_username: opts.username.clone(),
        }));
        databases.ensure_configuration(&opts.username).await?;

        Ok(databases)
    }

    /// Ensure the pgbouncer user is setup and the connecting user has the correct permissions
    #[instrument(skip(self))]
    async fn ensure_configuration(&self, connecting_user: &str) -> Result<()> {
        let default = self.get_default().await?;

        // Ensure the connecting user has the correct permissions
        let connecting_user = query_file_as!(User, "queries/user-permissions.sql", connecting_user)
            .fetch_one(&default)
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
            .fetch_optional(&default)
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
                query!("CREATE USER pgbouncer WITH LOGIN NOSUPERUSER NOCREATEROLE NOCREATEDB NOREPLICATION NOBYPASSRLS")
                    .execute(&default)
                    .await?;
            }
        }

        // Setup the default database for pgbouncer authentication just in case
        ensure_schema(&default).await?;
        ensure_authentication_query(&default).await?;

        Ok(())
    }

    /// Get a list of all the managed databases
    pub fn managed_databases(&self) -> Vec<String> {
        let pools = self.0.pools.read();
        pools.keys().map(|d| d.to_owned()).collect()
    }

    /// Get a connection to the default database
    #[instrument(skip_all)]
    pub(crate) async fn get_default(&self) -> Result<PgPool> {
        self.get(&self.0.default_dbname).await
    }

    /// Get a connection to the specified database
    #[instrument(skip(self))]
    async fn get(&self, database: &str) -> Result<PgPool> {
        {
            let pools = self.0.pools.read();
            if let Some(pool) = pools.get(database) {
                return Ok(pool.clone());
            }
        }

        let pool = self.open(database).await?;
        let stored = pool.clone();

        {
            let mut pools = self.0.pools.write();
            pools.insert(database.to_string(), stored);
        }

        Ok(pool)
    }

    /// Open a new connection to the database
    #[instrument(skip(self))]
    async fn open(&self, database: &str) -> Result<PgPool> {
        let options = self.0.options.clone().database(database);

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

    /// Ensure the specified database exists and is configured properly
    #[instrument(skip(self, password))]
    pub async fn ensure(&self, database: &str, password: &str) -> Result<()> {
        if database == self.0.default_dbname {
            return Err(Error::DefaultDatabase);
        }

        // Setup the database and corresponding user
        let default = self.get_default().await?;
        ensure_user(database, password, &default).await?;
        ensure_database(database, &default).await?;
        info!("setup database and user");

        // Configure the database for authentication
        let connection = self.get(database).await?;
        ensure_schema(&connection).await?;
        ensure_authentication_query(&connection).await?;

        Ok(())
    }

    /// Remove a database from being managed. If `retain` is true, the database will not be dropped.
    #[instrument]
    pub async fn remove(&self, database: &str, retain: bool) -> Result<()> {
        if database == self.0.default_dbname {
            return Err(Error::DefaultDatabase);
        }

        let pool = match {
            let mut pools = self.0.pools.write();
            pools.remove(database)
        } {
            Some(p) => p,
            None => return Ok(()),
        };

        pool.close().await;

        let default = self.get_default().await?;

        let sql = if retain {
            format!(
                "ALTER DATABASE {database} OWNER TO {}",
                &self.0.default_username
            )
        } else {
            format!("DROP DATABASE {database}")
        };
        query(&sql).execute(&default).await?;
        info!("removed database");

        // Remove the user
        query(&format!("DROP USER {database}"))
            .execute(&default)
            .await?;
        info!("removed user");

        Ok(())
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

/// Ensure the user exists with the given password
#[instrument(skip(password, pool))]
async fn ensure_user(name: &str, password: &str, pool: &PgPool) -> Result<()> {
    let user = query_file_as!(User, "queries/user-permissions.sql", name)
        .fetch_optional(pool)
        .await?;
    debug!(?user);

    let sql = match user {
        Some(_) => format!("ALTER USER {name} WITH PASSWORD '{password}'"),
        None => format!("CREATE USER {name} WITH LOGIN NOSUPERUSER NOCREATEROLE NOCREATEDB NOREPLICATION NOBYPASSRLS PASSWORD '{password}'"),
    };
    query(&sql).execute(pool).await?;
    info!("upserted user");

    Ok(())
}

/// Ensure the database exists
#[instrument(skip(pool))]
async fn ensure_database(name: &str, pool: &PgPool) -> Result<()> {
    let database = query!(
        "SELECT oid FROM pg_catalog.pg_database WHERE datname = $1",
        name
    )
    .fetch_optional(pool)
    .await?;
    debug!(exists = ?database.is_some());

    // Create the database or ensure it's owner is correct
    let sql = match database {
        Some(_) => format!("ALTER DATABASE {name} OWNER TO {name}"),
        None => format!("CREATE DATABASE {name} WITH OWNER {name}"),
    };
    query(&sql).execute(pool).await?;

    Ok(())
}

/// Ensure the pgbouncer schema exists and has the proper permissions
#[instrument(skip_all)]
async fn ensure_schema(pool: &PgPool) -> Result<()> {
    query!("CREATE SCHEMA IF NOT EXISTS pgbouncer")
        .execute(pool)
        .await?;

    query!("GRANT USAGE ON SCHEMA pgbouncer TO pgbouncer")
        .execute(pool)
        .await?;

    Ok(())
}

/// Ensure the authentication lookup function exists and has the proper permissions
#[instrument(skip_all)]
async fn ensure_authentication_query(pool: &PgPool) -> Result<()> {
    let user_lookup_function = query_file!("queries/authentication-query-exists.sql")
        .fetch_optional(pool)
        .await?;
    debug!(exists = ?user_lookup_function.is_some());
    if user_lookup_function.is_none() {
        query_file!("queries/authentication-query-function.sql")
            .execute(pool)
            .await?;
    }
    info!("created authentication lookup if not exists");

    query!("REVOKE ALL ON FUNCTION pgbouncer.user_lookup(text) FROM public, pgbouncer")
        .execute(pool)
        .await?;
    query!("GRANT EXECUTE ON FUNCTION pgbouncer.user_lookup(text) TO pgbouncer")
        .execute(pool)
        .await?;
    info!("updated lookup function permissions");

    Ok(())
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid permissions for connected user")]
    InvalidPermissions,
    #[error("cannot create or remove default database")]
    DefaultDatabase,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}
