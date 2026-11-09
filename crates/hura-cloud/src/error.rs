//! What a request that failed answers: a status, and `{"error": "..."}` in
//! words a person can be shown.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub struct Error {
    status: StatusCode,
    message: String,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Error {
            status,
            message: message.into(),
        }
    }

    pub fn bad(message: impl Into<String>) -> Self {
        Error::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized() -> Self {
        Error::new(StatusCode::UNAUTHORIZED, "not signed in")
    }

    pub fn is_status(&self, status: StatusCode) -> bool {
        self.status == status
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Error::new(StatusCode::NOT_FOUND, message)
    }

    /// Something that is ours to fix. Logged with what went wrong; answered
    /// without it, since a database's error text is not for the client.
    pub fn internal(cause: impl std::fmt::Display) -> Self {
        eprintln!("error: {cause}");
        Error::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "something went wrong on the server",
        )
    }
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Error::internal(e)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
