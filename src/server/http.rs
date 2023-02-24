use axum::{http::Request, Router};
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, MakeSpan, TraceLayer};
use tracing::{span, Level, Span};
use uuid::Uuid;

/// Build the router for the management interface
pub fn router() -> Router {
    Router::new().layer(
        TraceLayer::new_for_http()
            .make_span_with(MakeSpanWithId)
            .on_request(DefaultOnRequest::new().level(Level::INFO))
            .on_response(DefaultOnResponse::new().level(Level::INFO)),
    )
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MakeSpanWithId;

impl<B> MakeSpan<B> for MakeSpanWithId {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        span!(
            Level::INFO,
            "external-postgres::request",
            method = %request.method(),
            uri = %request.uri(),
            version = ?request.version(),
            id = %Uuid::new_v4(),
        )
    }
}
