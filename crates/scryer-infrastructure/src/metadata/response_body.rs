use scryer_application::{AppError, AppResult};

pub(crate) const RESPONSE_BODY_PREVIEW_LIMIT_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseBodyPreview {
    pub text: String,
    pub preview_bytes: usize,
    pub content_length: Option<u64>,
    pub content_type: Option<String>,
    pub truncated: bool,
}

impl ResponseBodyPreview {
    pub(crate) fn from_text(text: &str) -> Self {
        Self::from_bytes(
            text.as_bytes(),
            None,
            None,
            text.len() > RESPONSE_BODY_PREVIEW_LIMIT_BYTES,
        )
    }

    fn from_bytes(
        bytes: &[u8],
        content_length: Option<u64>,
        content_type: Option<String>,
        truncated: bool,
    ) -> Self {
        let preview_len = bytes.len().min(RESPONSE_BODY_PREVIEW_LIMIT_BYTES);
        let text = String::from_utf8_lossy(&bytes[..preview_len]).into_owned();
        let truncated = truncated
            || bytes.len() > RESPONSE_BODY_PREVIEW_LIMIT_BYTES
            || content_length
                .map(|len| len > preview_len as u64)
                .unwrap_or(false);

        Self {
            text,
            preview_bytes: preview_len,
            content_length,
            content_type,
            truncated,
        }
    }

    pub(crate) fn escaped_text(&self) -> String {
        self.text.escape_debug().collect()
    }
}

pub(crate) async fn read_response_body_preview(
    mut response: reqwest::Response,
    read_context: &str,
) -> AppResult<ResponseBodyPreview> {
    let content_length = response.content_length();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let mut bytes = Vec::new();
    let mut truncated = false;

    while bytes.len() < RESPONSE_BODY_PREVIEW_LIMIT_BYTES {
        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| AppError::Repository(format!("{read_context}: {error}")))?
        else {
            break;
        };

        let remaining = RESPONSE_BODY_PREVIEW_LIMIT_BYTES - bytes.len();
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }

        bytes.extend_from_slice(&chunk);
    }

    if bytes.len() == RESPONSE_BODY_PREVIEW_LIMIT_BYTES
        && content_length
            .map(|len| len > RESPONSE_BODY_PREVIEW_LIMIT_BYTES as u64)
            .unwrap_or(true)
    {
        truncated = true;
    }

    Ok(ResponseBodyPreview::from_bytes(
        &bytes,
        content_length,
        content_type,
        truncated,
    ))
}

#[cfg(test)]
mod tests {
    use super::{RESPONSE_BODY_PREVIEW_LIMIT_BYTES, ResponseBodyPreview};

    #[test]
    fn preview_escapes_multiline_html_for_single_line_logging() {
        let preview = ResponseBodyPreview::from_text("<!DOCTYPE html>\n<html>bad gateway</html>");

        assert_eq!(
            preview.escaped_text(),
            "<!DOCTYPE html>\\n<html>bad gateway</html>"
        );
        assert!(!preview.truncated);
    }

    #[test]
    fn preview_truncates_large_bodies() {
        let body = "x".repeat(RESPONSE_BODY_PREVIEW_LIMIT_BYTES + 10);
        let preview = ResponseBodyPreview::from_text(&body);

        assert_eq!(preview.preview_bytes, RESPONSE_BODY_PREVIEW_LIMIT_BYTES);
        assert_eq!(preview.text.len(), RESPONSE_BODY_PREVIEW_LIMIT_BYTES);
        assert!(preview.truncated);
    }
}
