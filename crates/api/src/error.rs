use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use tomo_shared::ErrorBody;

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Forbidden(&'static str),
    BadRequest(String),
    Conflict(&'static str),
    NotFound(&'static str),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Not signed in.".to_string()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m.to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m.to_string()),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m.to_string()),
            ApiError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.".to_string()),
        };
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        // Log the real cause server-side; never leak database details to clients.
        tracing::error!("database error: {e}");
        ApiError::Internal
    }
}
