use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Shared HTTP executor configuration for remote observers.
#[derive(Debug, Clone)]
pub struct HttpExecutorConfig {
    /// Maximum number of in-flight tasks spawned by higher-level observers.
    pub max_in_flight: usize,
    /// Minimum delay between requests to the same host.
    pub per_host_min_interval: Duration,
    /// Number of request attempts before giving up.
    pub max_attempts: usize,
    /// Base delay used when backing off retryable requests.
    pub retry_base_delay: Duration,
    /// Connect timeout for the underlying client.
    pub connect_timeout: Duration,
    /// Total request timeout for the underlying client.
    pub request_timeout: Duration,
}

impl Default for HttpExecutorConfig {
    fn default() -> Self {
        Self {
            max_in_flight: 4,
            per_host_min_interval: Duration::from_millis(250),
            max_attempts: 3,
            retry_base_delay: Duration::from_millis(250),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
        }
    }
}

/// Result of a completed HTTP execution attempt.
#[derive(Debug)]
pub struct ExecutedRequest {
    /// Final response returned by the executor.
    pub response: reqwest::Response,
}

#[derive(Debug, Default)]
struct HttpState {
    next_allowed_by_host: Mutex<HashMap<String, Instant>>,
}

/// Shared paced and retrying HTTP request executor.
#[derive(Debug, Clone)]
pub struct HttpExecutor {
    client: reqwest::Client,
    config: HttpExecutorConfig,
    state: Arc<HttpState>,
}

impl HttpExecutor {
    /// Builds a new executor.
    pub fn new(config: HttpExecutorConfig) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()?;
        Ok(Self {
            client,
            config,
            state: Arc::new(HttpState::default()),
        })
    }

    /// Returns the shared reqwest client.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Returns the configured maximum number of in-flight tasks.
    pub fn max_in_flight(&self) -> usize {
        self.config.max_in_flight.max(1)
    }

    /// Returns a host label for pacing derived from a URL.
    pub fn host_for_url(&self, url: &str) -> String {
        reqwest::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Executes a paced request with limited retries for retryable failures.
    pub async fn execute<F>(&self, host: &str, build: F) -> Option<ExecutedRequest>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        for attempt in 1..=self.config.max_attempts.max(1) {
            self.pace_host(host).await;
            let result = build().send().await;
            match result {
                Ok(response) if self.should_retry_status(response.status()) => {
                    if attempt == self.config.max_attempts {
                        return Some(ExecutedRequest { response });
                    }
                    let retry_delay = self
                        .retry_after(&response)
                        .unwrap_or_else(|| self.backoff_delay(attempt));
                    tokio::time::sleep(retry_delay).await;
                }
                Ok(response) => {
                    return Some(ExecutedRequest { response });
                }
                Err(_) if attempt < self.config.max_attempts => {
                    tokio::time::sleep(self.backoff_delay(attempt)).await;
                }
                Err(_) => return None,
            }
        }
        None
    }

    fn should_retry_status(&self, status: reqwest::StatusCode) -> bool {
        status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
    }

    fn retry_after(&self, response: &reqwest::Response) -> Option<Duration> {
        let raw = response.headers().get(reqwest::header::RETRY_AFTER)?;
        let raw = raw.to_str().ok()?;
        raw.parse::<u64>().ok().map(Duration::from_secs)
    }

    fn backoff_delay(&self, attempt: usize) -> Duration {
        let factor = 1u32
            .checked_shl(attempt.saturating_sub(1).min(8) as u32)
            .unwrap_or(u32::MAX);
        self.config.retry_base_delay.saturating_mul(factor)
    }

    async fn pace_host(&self, host: &str) {
        let now = Instant::now();
        let sleep_for = {
            let mut guard = self
                .state
                .next_allowed_by_host
                .lock()
                .expect("http pacing lock poisoned");
            let entry = guard.entry(host.to_string()).or_insert(now);
            let scheduled = (*entry).max(now);
            *entry = scheduled + self.config.per_host_min_interval;
            scheduled.saturating_duration_since(now)
        };
        if !sleep_for.is_zero() {
            tokio::time::sleep(sleep_for).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HttpExecutor, HttpExecutorConfig};

    #[test]
    fn derives_host_labels_from_urls() {
        let executor = HttpExecutor::new(HttpExecutorConfig::default()).expect("executor");
        assert_eq!(
            executor.host_for_url("https://index.crates.io/config.json"),
            "index.crates.io"
        );
        assert_eq!(executor.host_for_url("not-a-url"), "unknown");
    }
}
