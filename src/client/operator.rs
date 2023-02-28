use crate::{
    constants::APPLICATION_NAME,
    models::operator::{ChangeStateRequest, ChangeStateResponse, StateResponse, Status},
};
use clap::Subcommand;
use eyre::WrapErr;
use reqwest::Client;
use tracing::{info, warn};
use url::Url;

#[derive(Debug, Subcommand)]
#[command(rename_all = "kebab-case")]
pub enum Command {
    /// Enable the operator
    Enable,
    /// Disable the operator
    Disable,
    /// Check whether the operator is running
    Status,
}

pub async fn client(address: Url, command: Command) -> eyre::Result<()> {
    let client = Client::builder().user_agent(APPLICATION_NAME).build()?;

    match command {
        Command::Enable => change_state(address, Status::Enabled, client).await,
        Command::Disable => change_state(address, Status::Disabled, client).await,
        Command::Status => get_state(address, client).await,
    }
}

async fn change_state(address: Url, desired: Status, client: Client) -> eyre::Result<()> {
    let response = client
        .post(address.join("/operator/state")?)
        .json(&ChangeStateRequest { desired })
        .send()
        .await
        .wrap_err("failed to send request")?
        .error_for_status()
        .wrap_err("unexpected status code")?
        .json::<ChangeStateResponse>()
        .await?;

    match (desired, response.success) {
        (Status::Enabled, true) => info!("successfully enabled operator"),
        (Status::Enabled, false) => warn!("failed to enable operator, check server logs"),
        (Status::Disabled, _) => info!("successfully disabled operator"),
    }

    Ok(())
}

async fn get_state(address: Url, client: Client) -> eyre::Result<()> {
    let response: StateResponse = client
        .get(address.join("/operator/state")?)
        .send()
        .await
        .wrap_err("failed to send request")?
        .error_for_status()
        .wrap_err("unexpected status code")?
        .json()
        .await?;

    match response.running {
        true => info!("operator is running"),
        false => info!("operator is stopped"),
    }

    Ok(())
}
