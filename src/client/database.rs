use crate::{
    constants::APPLICATION_NAME,
    models::database::{CreateRequest, CreateResponse, DeleteOptions},
};
use clap::Subcommand;
use eyre::{bail, WrapErr};
use reqwest::{Client, StatusCode};
use tracing::info;
use url::Url;

#[derive(Debug, Subcommand)]
#[command(rename_all = "kebab-case")]
pub enum Command {
    /// Get a list of all the managed databases
    List,
    /// Create a new managed database
    Create {
        /// The database's name
        name: String,
    },
    /// Ensure a database is configured correctly
    Check {
        /// The database's name
        name: String,
    },
    /// Remove a database from management
    Remove {
        /// The database's name
        name: String,
        /// Whether to retain the database's contents
        #[arg(long)]
        retain: bool,
    },
}

pub async fn client(address: Url, command: Command) -> eyre::Result<()> {
    let client = Client::builder().user_agent(APPLICATION_NAME).build()?;

    let request = match &command {
        Command::List => client.get(address.join("/databases")?).build(),
        Command::Create { name } => client
            .post(address.join("/databases")?)
            .json(&CreateRequest { name: name.clone() })
            .build(),
        Command::Check { name } => client
            .put(address.join(&format!("/databases/{name}"))?)
            .build(),
        Command::Remove { name, retain } => client
            .delete(address.join(&format!("/databases/{name}"))?)
            .query(&DeleteOptions {
                retain: Some(*retain),
            })
            .build(),
    }
    .wrap_err("failed to build request")?;

    let response = client
        .execute(request)
        .await
        .wrap_err("failed to send request")?;

    if response.status() == StatusCode::NOT_FOUND {
        bail!("database not found");
    }
    let response = response
        .error_for_status()
        .wrap_err("unexpected status code")?;

    match command {
        Command::List => {
            let databases = response.json::<Vec<String>>().await?;
            info!(?databases);
        }
        Command::Create { .. } => {
            let created = response.json::<CreateResponse>().await?;
            match created.password {
                Some(password) => info!(%password, "created database"),
                None => info!("database already exists"),
            }
        }
        _ => info!("successfully enqueued operation"),
    }

    Ok(())
}
