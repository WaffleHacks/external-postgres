use crate::{constants::APPLICATION_NAME, models::ErrorResponse};
use eyre::WrapErr;
use reqwest::{Client, StatusCode};
use tracing::{info, warn};
use url::Url;

mod database;
mod operator;

pub use database::{client as database, Command as DatabaseCommand};
pub use operator::{client as operator, Command as OperatorCommand};

pub async fn health(address: Url) -> eyre::Result<()> {
    let client = Client::builder().user_agent(APPLICATION_NAME).build()?;

    let response = client
        .get(address.join("/health")?)
        .send()
        .await
        .wrap_err("failed to send request")?;

    if response.status() == StatusCode::NO_CONTENT {
        info!("healthy");
    } else {
        let error = response.json::<ErrorResponse>().await?;
        warn!(status = %error.code, reason = %error.message, "unhealthy");
    }

    Ok(())
}
