use crate::server::{controller::Controller, database::Databases};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

#[instrument(name = "database_list")]
pub async fn list(State(databases): State<Databases>) -> Json<Vec<String>> {
    Json(databases.managed_databases())
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    password: Option<String>,
}

#[instrument(name = "database_create")]
pub async fn create(
    State(controller): State<Controller>,
    Json(request): Json<CreateRequest>,
) -> Json<CreateResponse> {
    let password = controller.create(request.name).await;
    Json(CreateResponse { password })
}

#[derive(Debug, Deserialize)]
pub struct DeleteOptions {
    retain: Option<bool>,
}

#[instrument(name = "database_delete")]
pub async fn delete(
    Path(name): Path<String>,
    Query(options): Query<DeleteOptions>,
    State(controller): State<Controller>,
) -> StatusCode {
    controller.remove(name, options.retain.unwrap_or_default());

    StatusCode::NO_CONTENT
}
