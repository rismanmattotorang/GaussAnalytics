//! Mapping from [`CoreError`] to HTTP responses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use gauss_core::error::CoreError;
use serde_json::json;

/// Newtype wrapper so we can implement `IntoResponse` for the workspace error.
#[derive(Debug)]
pub struct ApiError(pub CoreError);

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            CoreError::NotFound(_) => StatusCode::NOT_FOUND,
            CoreError::PermissionDenied(_) => StatusCode::FORBIDDEN,
            CoreError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            CoreError::InvalidQuery(_) | CoreError::Compilation(_) | CoreError::Config(_) => {
                StatusCode::BAD_REQUEST
            }
            CoreError::Integration(_) => StatusCode::BAD_GATEWAY,
            CoreError::Storage(_) | CoreError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}
