use crate::server::database;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

pub(crate) type Result<T> = std::result::Result<T, Error>;

/// An API error
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] database::Error),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let message = format!("{self}");
        let code = match self {
            Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let mut response = Json(ErrorResponse {
            code: code.as_u16(),
            message,
        })
        .into_response();

        *response.status_mut() = code;
        response
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    code: u16,
    message: String,
}
