use reqwest::{Error, StatusCode};
use std::io;
use std::thread;
use std::time::Duration;

pub(super) const MAX_ATTEMPTS: usize = 3;
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);

/// Statuses that are normally transient for idempotent GET requests.
pub(super) fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_EARLY | StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
}

/// A GET can be sent again when reqwest knows that the failure was transport
/// related. Body errors include a response whose stream was interrupted.
pub(super) fn retryable_get_error(error: &Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request() || error.is_body()
}

/// A multipart POST is replayable only when the connection could not be
/// established. Timeouts and body errors are deliberately ambiguous: the
/// server may already have received and imported the project.
pub(super) fn safe_upload_retry(error: &Error) -> bool {
    should_retry_upload(error.is_connect())
}

pub(super) fn should_retry_upload(connection_not_established: bool) -> bool {
    connection_not_established
}

pub(super) fn retryable_body_error(_error: &io::Error) -> bool {
    // A blocking response body error means the GET stream was interrupted;
    // unlike local file writes, replaying the idempotent GET is safe.
    true
}

pub(super) fn backoff(attempt: usize) -> Duration {
    // The retry budget is three attempts; cap the helper for future callers.
    let multiplier = 1_u32
        .checked_shl(attempt.min(15) as u32)
        .unwrap_or(u32::MAX);
    INITIAL_BACKOFF.saturating_mul(multiplier)
}

pub(super) fn wait_before_retry(failed_attempt: usize) {
    thread::sleep(backoff(failed_attempt));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_budget_and_backoff_are_bounded_and_deterministic() {
        assert_eq!(MAX_ATTEMPTS, 3);
        assert_eq!(backoff(0), Duration::from_millis(250));
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_millis(1000));
        assert!(backoff(100) >= backoff(2));
    }

    #[test]
    fn only_transient_get_statuses_are_retryable() {
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_EARLY,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(retryable_status(status));
        }
        for status in [
            StatusCode::OK,
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::MOVED_PERMANENTLY,
        ] {
            assert!(!retryable_status(status));
        }
    }

    #[test]
    fn upload_retry_policy_only_replays_known_no_request_failures() {
        assert!(should_retry_upload(true));
        assert!(!should_retry_upload(false));
    }
}
