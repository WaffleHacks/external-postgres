use super::error::Result;
use crate::{
    models::database::{CreateRequest, DeleteOptions},
    server::database::Databases,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use tracing::instrument;

#[instrument(name = "database_list", skip_all)]
pub async fn list(State(databases): State<Databases>) -> Json<Vec<String>> {
    Json(databases.managed_databases())
}

#[instrument(name = "database_ensure", skip(databases))]
pub async fn ensure(
    State(databases): State<Databases>,
    Json(request): Json<CreateRequest>,
) -> Result<StatusCode> {
    databases.ensure(&request.name, &request.password).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(name = "database_delete", skip(databases))]
pub async fn delete(
    Path(name): Path<String>,
    Query(options): Query<DeleteOptions>,
    State(databases): State<Databases>,
) -> Result<StatusCode> {
    databases
        .remove(&name, options.retain.unwrap_or_default())
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
