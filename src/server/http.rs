use super::database::Databases;
use axum::{
    extract::{FromRef, State},
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, MakeSpan, TraceLayer};
use tracing::{span, Level, Span};
use uuid::Uuid;

/// Build the router for the management interface
pub fn router(databases: Databases) -> Router {
    Router::new()
        .route("/health", get(health))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(MakeSpanWithId)
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(AppState { databases })
}

#[derive(Clone)]
pub struct AppState {
    databases: Databases,
}

impl FromRef<AppState> for Databases {
    fn from_ref(input: &AppState) -> Self {
        input.databases.clone()
    }
}

async fn health(State(databases): State<Databases>) -> StatusCode {
    let default = databases.get_default().await.unwrap();
    default.ping().await.unwrap();

    StatusCode::NO_CONTENT
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
