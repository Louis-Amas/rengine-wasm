use rengine_metrics::{counters::increment_counter, latencies::record_latency};
use std::{
    error::Error,
    fmt::{self, Display},
    time::Instant,
};

/// Extension utility for [`reqwest::Response`]s.
pub trait ResponseExt: Sized {
    /// Returns `Ok(self)` if the response is a 2xx success.
    /// Otherwise returns an Err including the url, status and response body.
    ///
    /// Note: Similar to [`reqwest::Response::error_for_status`] but enforces
    /// status=2xx and reads the body for inclusion in errors
    #[allow(async_fn_in_trait)]
    async fn ok_status(self) -> Result<Self, NotOkResponse>;
}

impl ResponseExt for reqwest::Response {
    async fn ok_status(self) -> Result<Self, NotOkResponse> {
        let status = self.status();
        if status.is_success() {
            return Ok(self);
        }

        Err(NotOkResponse {
            url: self.url().clone(),
            status,
            body: self.text().await.unwrap_or_default(),
        })
    }
}

#[derive(Debug)]
pub struct NotOkResponse {
    url: reqwest::Url,
    status: reqwest::StatusCode,
    body: String,
}

impl Display for NotOkResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} not ok {}", self.url, self.status.as_u16())?;
        if !self.body.is_empty() {
            write!(f, ": {}", self.body)?;
        }
        Ok(())
    }
}

impl Error for NotOkResponse {}

pub trait RequestExt {
    /// Builds and sends this request. Returns a 2xx response or error.
    ///
    /// Convenience for [`reqwest::RequestBuilder::send`] + [`ResponseExt::ok_status`].
    #[allow(async_fn_in_trait)]
    async fn send_ok(self, label: &str) -> anyhow::Result<reqwest::Response>;
}

impl RequestExt for reqwest::RequestBuilder {
    async fn send_ok(self, label: &str) -> anyhow::Result<reqwest::Response> {
        let start = Instant::now();
        let res = self.send().await;

        let path = match &res {
            Ok(response) => response.url().path().to_string(),
            Err(e) => e
                .url()
                .map(|u| u.path().to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        };

        record_latency(&path, start);
        increment_counter(format!("{}|call", label));

        match res {
            Ok(response) => match response.ok_status().await {
                Ok(response) => {
                    increment_counter(format!("{}|success", label));
                    Ok(response)
                }
                Err(e) => {
                    increment_counter(format!("{}|failure", label));
                    tracing::error!(?e, label, path, "HTTP request failed");
                    Err(e.into())
                }
            },
            Err(e) => {
                increment_counter(format!("{}|network_error", label));
                tracing::error!(?e, label, path, "HTTP network error");
                Err(e.into())
            }
        }
    }
}
