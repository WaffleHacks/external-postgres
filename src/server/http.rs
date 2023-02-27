use super::{database::Databases, operator::Operator};
use axum::{
    extract::{FromRef, State},
    http::{Request, StatusCode},
    routing::{delete, get},
    Router,
};
use sqlx::query;
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, MakeSpan, TraceLayer};
use tracing::{span, Level, Span};
use uuid::Uuid;

mod database;
mod error;

#[derive(Clone)]
pub struct AppState {
    databases: Databases,
    operator: Operator,
}

impl FromRef<AppState> for Databases {
    fn from_ref(input: &AppState) -> Self {
        input.databases.clone()
    }
}

/// Build the router for the management interface
pub fn router(databases: Databases, operator: Operator) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/databases", get(database::list).post(database::ensure))
        .route("/databases/:database", delete(database::delete))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(MakeSpanWithId)
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(AppState {
            databases,
            operator,
        })
}

async fn health(State(databases): State<Databases>) -> error::Result<StatusCode> {
    let default = databases.get_default().await?;
    query!("SELECT 1 as test").fetch_one(&default).await?;
    // default.ping().await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MakeSpanWithId;

impl<B> MakeSpan<B> for MakeSpanWithId {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        span!(
            Level::INFO,
            "external_postgres::request",
            method = %request.method(),
            uri = %request.uri(),
            version = ?request.version(),
            id = %Uuid::new_v4(),
        )
    }
}
