use super::database::Databases;
use std::fmt::{Debug, Formatter};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot::{self, Sender},
    },
    task::JoinHandle,
};
use tracing::{debug, error, info, instrument};

/// Manages the lifecycle of all databases
#[derive(Clone, Debug)]
pub struct Controller {
    sender: UnboundedSender<Command>,
}

impl Controller {
    /// Create and start the controller
    pub fn start(databases: Databases) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel::<Command>();

        let handle = tokio::spawn(processor(databases, rx));

        (Controller { sender: tx }, handle)
    }

    fn send(&self, command: Command) {
        if let Err(error) = self.sender.send(command) {
            error!(%error, "failed to send command")
        }
    }

    /// Create a new database
    pub async fn create(&self, name: String) -> Option<String> {
        let (tx, rx) = oneshot::channel::<Option<String>>();
        self.send(Command::Create {
            database: name,
            result: tx,
        });

        rx.await.unwrap()
    }

    /// Check the database is setup correctly
    pub fn check(&self, name: String) {
        self.send(Command::Check(name))
    }

    /// Remove a database, optionally retaining its data
    pub fn remove(&self, name: String, retain: bool) {
        self.send(Command::Remove {
            database: name,
            retain,
        })
    }

    /// Stop the controller
    pub fn stop(self) {
        self.send(Command::Halt)
    }
}

/// A command sent to the controller
enum Command {
    Create {
        database: String,
        result: Sender<Option<String>>,
    },
    Check(String),
    Remove {
        database: String,
        retain: bool,
    },
    Halt,
}

impl Debug for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::Create { database, .. } => f
                .debug_struct("Create")
                .field("database", database)
                .finish(),
            Command::Check(database) => {
                f.debug_struct("Check").field("database", database).finish()
            }
            Command::Remove { database, retain } => f
                .debug_struct("Remove")
                .field("database", database)
                .field("retain", retain)
                .finish(),
            Command::Halt => write!(f, "Halt"),
        }
    }
}

#[instrument(skip_all)]
async fn processor(databases: Databases, mut rx: UnboundedReceiver<Command>) {
    while let Some(command) = rx.recv().await {
        if handle_command(&databases, command).await {
            break;
        }
    }
}

macro_rules! fail {
    ($result:expr) => {
        {
            use std::error::Error;

            match $result {
                Ok(v) => v,
                Err(error) => {
                    tracing::error!(%error, source = ?error.source(), "failed to process command");
                    return false;
                }
            }
        }
    };
}

#[instrument(skip(databases), name = "command")]
async fn handle_command(databases: &Databases, command: Command) -> bool {
    info!("new command received");

    match command {
        Command::Create { database, result } => {
            let (pool, password) = fail!(databases.ensure_exists(&database).await);
            // So long as the database and user are created successfully, send back the password
            result.send(password).unwrap();

            fail!(pool.ensure_schema().await);
            debug!("schema exists");
            fail!(pool.ensure_authentication_query().await);
            debug!("authentication query exists");
        }
        Command::Check(database) => {
            let pool = fail!(databases.get(&database).await);
            fail!(pool.ensure_schema().await);
            debug!("schema exists");
            fail!(pool.ensure_authentication_query().await);
            debug!("authentication query exists");
        }
        Command::Remove { database, retain } => {
            fail!(databases.remove(&database, retain).await);
        }
        Command::Halt => return true,
    }

    info!("command completed");

    false
}
