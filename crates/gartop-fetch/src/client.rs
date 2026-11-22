//! Blocking HTTP client for the Garmin Personal API.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::auth::TokenProvider;

/// Errors from talking to the API.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("HTTP {status} from {url}: {body}")]
    Status {
        status: u16,
        url: String,
        body: String,
    },
    #[error("request to {url} failed: {source}")]
    Transport {
        url: String,
        source: Box<ureq::Error>,
    },
    #[error("invalid JSON from {url}: {source}")]
    Json {
        url: String,
        source: serde_json::Error,
    },
    #[error("invalid access token: {0}")]
    BadToken(String),
}

impl ApiError {
    /// HTTP status code, if the error came from a non-2xx response.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// Thin wrapper around a `ureq` agent bound to a base URL, with request pacing.
#[derive(Debug)]
pub struct Http {
    agent: ureq::Agent,
    base_url: String,
    min_interval: Duration,
    last_request: std::cell::Cell<Option<Instant>>,
}

impl Http {
    #[must_use]
    pub fn new(base_url: &str, min_interval: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("gartop/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            agent,
            base_url: base_url.trim_end_matches('/').to_owned(),
            min_interval,
            last_request: std::cell::Cell::new(None),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn pace(&self) {
        if let Some(last) = self.last_request.get()
            && let Some(wait) = self.min_interval.checked_sub(last.elapsed())
        {
            std::thread::sleep(wait);
        }
        self.last_request.set(Some(Instant::now()));
    }

    fn finish(
        url: String,
        resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<String, ApiError> {
        let mut resp = resp.map_err(|e| ApiError::Transport {
            url: url.clone(),
            source: Box::new(e),
        })?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| ApiError::Transport {
                url: url.clone(),
                source: Box::new(e),
            })?;
        if (200..300).contains(&status) {
            Ok(body)
        } else {
            let mut body = body;
            body.truncate(300);
            Err(ApiError::Status { status, url, body })
        }
    }

    /// `POST` with `Authorization: ApiKey <key>`, returning the body text.
    pub fn post_api_key(&self, path: &str, api_key: &str) -> Result<String, ApiError> {
        self.pace();
        let url = self.url(path);
        let resp = self
            .agent
            .post(&url)
            .header("Authorization", &format!("ApiKey {api_key}"))
            .header("Accept", "*/*")
            .send_empty();
        Self::finish(url, resp)
    }

    /// `GET` with a bearer token and query parameters, returning the body text.
    pub fn get_bearer(
        &self,
        path: &str,
        query: &[(&str, String)],
        jwt: &str,
    ) -> Result<String, ApiError> {
        self.pace();
        let url = self.url(path);
        let mut req = self
            .agent
            .get(&url)
            .header("Authorization", &format!("Bearer {jwt}"))
            .header("Accept", "application/json");
        for (k, v) in query {
            req = req.query(*k, v);
        }
        Self::finish(url, req.call())
    }
}

/// Authenticated Garmin API client.
#[derive(Debug)]
pub struct GarminClient {
    http: Http,
    tokens: TokenProvider,
}

impl GarminClient {
    #[must_use]
    pub fn new(http: Http, tokens: TokenProvider) -> Self {
        Self { http, tokens }
    }

    /// `GET` a path and return the body as text, renewing the token once on a 401.
    pub fn get_text(&mut self, path: &str, query: &[(&str, String)]) -> Result<String, ApiError> {
        let jwt = self.tokens.token(&self.http)?.jwt.clone();
        match self.http.get_bearer(path, query, &jwt) {
            Err(e) if e.status() == Some(401) => {
                tracing::info!("access token rejected, requesting a new one");
                let jwt = self.tokens.renew(&self.http)?.jwt.clone();
                self.http.get_bearer(path, query, &jwt)
            }
            other => other,
        }
    }

    /// `GET` a path and parse the body as JSON. An empty body becomes `null`.
    pub fn get_json(&mut self, path: &str, query: &[(&str, String)]) -> Result<Value, ApiError> {
        let body = self.get_text(path, query)?;
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body).map_err(|source| ApiError::Json {
            url: path.to_owned(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{TOKEN_PATH, tests::fake_jwt};

    fn client(server: &mockito::Server, cache: &std::path::Path) -> GarminClient {
        let http = Http::new(&server.url(), Duration::ZERO);
        GarminClient::new(http, TokenProvider::new("k3y".into(), cache.to_path_buf()))
    }

    fn far_future() -> i64 {
        chrono::Utc::now().timestamp() + 86_400
    }

    #[test]
    fn token_is_fetched_once_and_reused() {
        let mut server = mockito::Server::new();
        let jwt = fake_jwt(far_future());
        let token = server
            .mock("POST", TOKEN_PATH)
            .match_header("authorization", "ApiKey k3y")
            .with_body(&jwt)
            .expect(1)
            .create();
        let data = server
            .mock("GET", "/x")
            .match_header("authorization", format!("Bearer {jwt}").as_str())
            .match_query(mockito::Matcher::UrlEncoded("a".into(), "1".into()))
            .with_body(r#"{"ok":true}"#)
            .expect(2)
            .create();
        let dir = tempfile::tempdir().unwrap();
        let mut c = client(&server, &dir.path().join("token"));
        for _ in 0..2 {
            let v = c.get_json("/x", &[("a", "1".into())]).unwrap();
            assert_eq!(v["ok"], true);
        }
        token.assert();
        data.assert();

        // A new client reuses the cached token without hitting the token endpoint.
        let mut c2 = client(&server, &dir.path().join("token"));
        c2.get_json("/x", &[("a", "1".into())]).unwrap();
        token.assert();
    }

    #[test]
    fn renews_token_once_after_401() {
        let mut server = mockito::Server::new();
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("token");
        // Seed the cache with a token the server will reject.
        let stale = fake_jwt(far_future() + 1);
        std::fs::write(
            &cache,
            serde_json::to_string(&crate::auth::Token::from_jwt(&stale).unwrap()).unwrap(),
        )
        .unwrap();
        let fresh = fake_jwt(far_future());
        let token = server
            .mock("POST", TOKEN_PATH)
            .with_body(&fresh)
            .expect(1)
            .create();
        let rejected = server
            .mock("GET", "/x")
            .match_header("authorization", format!("Bearer {stale}").as_str())
            .with_status(401)
            .expect(1)
            .create();
        let ok = server
            .mock("GET", "/x")
            .match_header("authorization", format!("Bearer {fresh}").as_str())
            .with_body("[1,2]")
            .expect(1)
            .create();
        let mut c = client(&server, &cache);
        assert_eq!(c.get_json("/x", &[]).unwrap(), serde_json::json!([1, 2]));
        token.assert();
        rejected.assert();
        ok.assert();
    }

    #[test]
    fn maps_errors() {
        let mut server = mockito::Server::new();
        server
            .mock("POST", TOKEN_PATH)
            .with_body(fake_jwt(far_future()))
            .create();
        server
            .mock("GET", "/missing")
            .with_status(404)
            .with_body(r#"{"error":"NOT_FOUND"}"#)
            .create();
        server.mock("GET", "/empty").with_body("").create();
        server.mock("GET", "/bad").with_body("not json").create();
        let dir = tempfile::tempdir().unwrap();
        let mut c = client(&server, &dir.path().join("token"));

        let err = c.get_json("/missing", &[]).unwrap_err();
        assert_eq!(err.status(), Some(404));
        assert!(err.to_string().contains("NOT_FOUND"));
        assert_eq!(c.get_json("/empty", &[]).unwrap(), Value::Null);
        assert!(matches!(
            c.get_json("/bad", &[]),
            Err(ApiError::Json { .. })
        ));
    }

    #[test]
    fn bad_api_key_surfaces_status() {
        let mut server = mockito::Server::new();
        server.mock("POST", TOKEN_PATH).with_status(401).create();
        let dir = tempfile::tempdir().unwrap();
        let mut c = client(&server, &dir.path().join("token"));
        assert_eq!(c.get_json("/x", &[]).unwrap_err().status(), Some(401));
    }
}
