use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_id: Option<String>,
}

impl ErrorResponse {
    pub(crate) fn new(error: String) -> Self {
        Self {
            error,
            error_id: None,
        }
    }

    pub(crate) fn with_error_id(error: String, error_id: String) -> Self {
        Self {
            error,
            error_id: Some(error_id),
        }
    }
}
