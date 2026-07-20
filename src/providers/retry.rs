use std::time::Duration;

use reqwest::{RequestBuilder, Response, StatusCode};

use crate::errors::GatewayError;

// Retries the initial request handshake (not the full response handling) on
// connection errors or a 429/5xx status, with exponential backoff. Rebuilds
// the request from scratch each attempt since `RequestBuilder` is consumed by
// `send()` and isn't `Clone`. Callers keep their existing "check status, read
// body on failure" code unchanged — a non-retryable response (or one that
// exhausted its retries) is returned as-is for the caller to handle.
pub async fn send_with_retry<F>(build: F, max_attempts: u32) -> Result<Response, GatewayError>
where
    F: Fn() -> RequestBuilder,
{
    let mut attempt = 0;

    loop {
        match build().send().await {
            Ok(response) if is_retryable_status(response.status()) && attempt < max_attempts => {
                attempt += 1;
                tokio::time::sleep(backoff_delay(attempt)).await;
            }
            Ok(response) => return Ok(response),
            Err(_) if attempt < max_attempts => {
                attempt += 1;
                tokio::time::sleep(backoff_delay(attempt)).await;
            }
            Err(e) => {
                return Err(GatewayError::Provider(format!("request failed: {}", e)));
            }
        }
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn backoff_delay(attempt: u32) -> Duration {
    Duration::from_millis(200 * 2u64.pow(attempt - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially() {
        assert_eq!(backoff_delay(1), Duration::from_millis(200));
        assert_eq!(backoff_delay(2), Duration::from_millis(400));
        assert_eq!(backoff_delay(3), Duration::from_millis(800));
    }

    #[test]
    fn retryable_statuses() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::OK));
    }
}
