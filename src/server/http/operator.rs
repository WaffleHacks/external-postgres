use crate::{
    models::operator::{ChangeStateRequest, ChangeStateResponse, StateResponse, Status},
    server::operator::Operator,
};
use axum::{extract::State, Json};
use tracing::instrument;

#[instrument(name = "operator_get_state", skip_all)]
pub async fn get_state(State(operator): State<Operator>) -> Json<StateResponse> {
    Json(StateResponse {
        running: operator.status(),
    })
}

#[instrument(name = "operator_change_state", skip_all, fields(desired = ?request.desired))]
pub async fn change_state(
    State(operator): State<Operator>,
    Json(request): Json<ChangeStateRequest>,
) -> Json<ChangeStateResponse> {
    let success = match request.desired {
        Status::Enabled => operator.start(),
        Status::Disabled => operator.stop().await,
    };

    Json(ChangeStateResponse { success })
}
