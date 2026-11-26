use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, DATE, RETRY_AFTER};
use reqwest::{Error, StatusCode};
use std::io;
use std::thread;
use std::time::{Duration, SystemTime};

pub(super) const MAX_ATTEMPTS: usize = 3;
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
// Do not block the migration indefinitely; a longer server delay is a failure,
// not permission to retry before the requested time.
const MAX_RESPONSE_WAIT: Duration = Duration::from_secs(30);

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

fn response_retry_delay(
    status: StatusCode,
    headers: &HeaderMap,
    failed_attempt: usize,
    now: SystemTime,
) -> Option<Duration> {
    if !retryable_status(status) || failed_attempt >= MAX_ATTEMPTS - 1 {
        return None;
    }
    let retry_after = headers.get_all(RETRY_AFTER);
    let mut values = retry_after.iter();
    let delay = match values.next() {
        None => backoff(failed_attempt),
        Some(value) => {
            // Ambiguous or malformed server instructions cannot safely be shortened.
            if values.next().is_some() {
                return None;
            }
            let value = value.to_str().ok()?.trim();
            let server_delay = if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
                Duration::from_secs(value.parse::<u64>().ok()?)
            } else {
                let deadline = DateTime::parse_from_rfc2822(value).ok()?.with_timezone(&Utc);
                let local_delay = deadline
                    .signed_duration_since(DateTime::<Utc>::from(now))
                    .to_std()
                    .unwrap_or_default();
                // A server Date helps avoid early retry when the local clock is ahead.
                let server_delay = headers
                    .get(DATE)
                    .and_then(|date| date.to_str().ok())
                    .and_then(|date| DateTime::parse_from_rfc2822(date).ok())
                    .map(|date| {
                        deadline
                            .signed_duration_since(date.with_timezone(&Utc))
                            .to_std()
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                local_delay.max(server_delay)
            };
            backoff(failed_attempt).max(server_delay)
        }
    };
    (delay <= MAX_RESPONSE_WAIT).then_some(delay)
}

/// Wait only when a GET response can be retried within the bounded budget.
pub(super) fn wait_for_response_retry(
    response: &reqwest::blocking::Response,
    failed_attempt: usize,
) -> bool {
    match response_retry_delay(
        response.status(),
        response.headers(),
        failed_attempt,
        SystemTime::now(),
    ) {
        Some(delay) => {
            thread::sleep(delay);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;
    use std::time::UNIX_EPOCH;

    #[test]
    fn response_retry_delay_respects_server_instructions_and_budget() {
        let now = UNIX_EPOCH + Duration::from_secs(1_447_841_800);
        let status = StatusCode::TOO_MANY_REQUESTS;
        let mut headers = HeaderMap::new();
        assert_eq!(response_retry_delay(status, &headers, 0, now), Some(backoff(0)));
        headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));
        assert_eq!(response_retry_delay(status, &headers, 0, now), Some(Duration::from_secs(5)));
        assert_eq!(response_retry_delay(status, &headers, 2, now), None);
        assert_eq!(response_retry_delay(StatusCode::OK, &headers, 0, now), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static("31"));
        assert_eq!(response_retry_delay(status, &headers, 0, now), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static("9999999999999999999999999"));
        assert_eq!(response_retry_delay(status, &headers, 0, now), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static("invalid"));
        assert_eq!(response_retry_delay(status, &headers, 0, now), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static("0"));
        headers.append(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(response_retry_delay(status, &headers, 0, now), None);
    }

    #[test]
    fn http_date_retry_after_accounts_for_server_clock() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("Wed, 21 Oct 2015 07:28:05 GMT"));
        headers.insert(DATE, HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"));
        let server_now = UNIX_EPOCH + Duration::from_secs(1_445_412_480);
        assert_eq!(
            response_retry_delay(StatusCode::TOO_MANY_REQUESTS, &headers, 0, server_now),
            Some(Duration::from_secs(5))
        );
        // Even if the local clock is ahead, use the server's five-second interval.
        assert_eq!(
            response_retry_delay(
                StatusCode::TOO_MANY_REQUESTS,
                &headers,
                0,
                server_now + Duration::from_secs(10),
            ),
            Some(Duration::from_secs(5))
        );
    }


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
