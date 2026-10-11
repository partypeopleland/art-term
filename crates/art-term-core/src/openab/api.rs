use super::{profiles::HostSnapshot, secret::Secret};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Method, StatusCode,
};
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ApiError {
    Auth,
    Tls,
    Transport,
    Timeout,
    Throttled,
    Capacity,
    SessionMissing,
    Protocol,
    StateChanged,
    Storage,
    UpgradeRequired,
    Replacement,
    SessionDead,
}
impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auth => "Authentication rejected",
            Self::Tls => "TLS verification failed",
            Self::Transport => "Transport failed",
            Self::Timeout => "Request timed out",
            Self::Throttled => "Request throttled",
            Self::Capacity => "Runtime capacity unavailable",
            Self::SessionMissing => "Session missing",
            Self::Protocol => "Invalid runtime protocol",
            Self::StateChanged => "Host or session state changed",
            Self::Storage => "Protected storage unavailable",
            Self::UpgradeRequired => "Runtime upgrade required",
            Self::Replacement => "Runtime or session replaced",
            Self::SessionDead => "Session exited",
        })
    }
}
impl std::error::Error for ApiError {}
fn network_error(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        return ApiError::Timeout;
    }
    // Inspect typed causes only; no request URL/header/body/error text reaches caller.
    if tls_cause(&error) {
        ApiError::Tls
    } else {
        ApiError::Transport
    }
}
pub(crate) fn tls_cause(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(e) = source {
        if e.downcast_ref::<rustls::Error>().is_some() {
            return true;
        }
        // io::Error::source delegates to its inner error's source, skipping the
        // actual payload. Descend get_ref explicitly through nested IO wrappers.
        source = if let Some(io) = e.downcast_ref::<std::io::Error>() {
            io.get_ref().map(|inner| inner as &dyn std::error::Error)
        } else {
            e.source()
        };
    }
    false
}
fn status_error(status: StatusCode) -> ApiError {
    match status.as_u16() {
        401 | 403 => ApiError::Auth,
        404 => ApiError::SessionMissing,
        409 => ApiError::StateChanged,
        429 => ApiError::Throttled,
        503 => ApiError::Capacity,
        _ => ApiError::Protocol,
    }
}
const ADMIN_REPLY_MAX: usize = 1024 * 1024;
pub fn validate_session_name(name: &str) -> Result<(), ApiError> {
    if !(1..=32).contains(&name.len())
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Err(ApiError::Protocol)
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub name: String,
    pub generation: u64,
    pub alive: bool,
    pub attached: bool,
    #[serde(alias = "bytes_written")]
    pub bytes_written: u64,
    pub tier: String,
    #[serde(alias = "teardown_best_effort")]
    pub teardown_best_effort: bool,
    #[serde(alias = "absolute_ttl_best_effort")]
    pub absolute_ttl_best_effort: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionList {
    pub sessions: Vec<SessionInfo>,
    pub draining: bool,
}
/// Native-only minted grant. Never sent through IPC or retained by AdminClient.
#[derive(Debug)]
pub struct SessionGrant {
    pub session: String,
    pub generation: u64,
    pub token: Secret,
    pub token_expires_in_secs: u64,
}
fn deserialize_secret<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Secret, D::Error> {
    let text = String::deserialize(d)?;
    Secret::new(text).map_err(|_| serde::de::Error::custom("invalid token"))
}
#[derive(Deserialize)]
struct WireGrant {
    session: String,
    generation: u64,
    #[serde(deserialize_with = "deserialize_secret")]
    token: Secret,
    token_expires_in_secs: u64,
}
/// No retained credential: caller lends an immutable store snapshot per operation.
#[derive(Clone)]
pub struct AdminClient {
    client: reqwest::Client,
}
impl AdminClient {
    fn secure_builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .https_only(true)
    }
    #[cfg(any(test, all(debug_assertions, feature = "openab-fixture")))]
    fn with_fixture_ca(ca_der: &[u8]) -> Result<Self, ApiError> {
        let ca = reqwest::Certificate::from_der(ca_der).map_err(|_| ApiError::Protocol)?;
        Ok(Self {
            client: Self::secure_builder()
                .add_root_certificate(ca)
                .build()
                .map_err(network_error)?,
        })
    }
    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
    pub fn fixture_ca(ca_der: &[u8]) -> Result<Self, ApiError> {
        Self::with_fixture_ca(ca_der)
    }
    #[cfg(test)]
    pub(super) fn tls_fixture(ca_der: &[u8]) -> Self {
        Self::with_fixture_ca(ca_der).unwrap()
    }
    #[cfg(test)]
    pub(super) fn fixture() -> Self {
        Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .no_proxy()
                .connect_timeout(Duration::from_secs(2))
                .read_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
        }
    }
    #[cfg(test)]
    pub(super) async fn fixture_request(
        &self,
        origin: &str,
        secret: &Secret,
    ) -> Result<Zeroizing<Vec<u8>>, ApiError> {
        self.request_at(
            origin,
            secret,
            Method::GET,
            "admin/sessions",
            None,
            ADMIN_REPLY_MAX,
            status_error,
        )
        .await
    }
    pub fn new() -> Result<Self, ApiError> {
        let client = Self::secure_builder().build().map_err(network_error)?;
        Ok(Self { client })
    }
    async fn request(
        &self,
        host: &HostSnapshot,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Zeroizing<Vec<u8>>, ApiError> {
        super::profiles::canonical_origin(&host.profile.base_url)?;
        self.request_at(
            &host.profile.base_url,
            &host.credential,
            method,
            path,
            body,
            ADMIN_REPLY_MAX,
            status_error,
        )
        .await
    }
    async fn request_at(
        &self,
        origin: &str,
        secret: &Secret,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        cap: usize,
        status_map: fn(StatusCode) -> ApiError,
    ) -> Result<Zeroizing<Vec<u8>>, ApiError> {
        let auth = Zeroizing::new(format!("Bearer {}", secret.expose()));
        let mut header = HeaderValue::from_str(&auth).map_err(|_| ApiError::Protocol)?;
        header.set_sensitive(true);
        let url = url::Url::parse(origin)
            .map_err(|_| ApiError::Protocol)?
            .join(path)
            .map_err(|_| ApiError::Protocol)?;
        let mut request = self
            .client
            .request(method, url)
            .header(AUTHORIZATION, header);
        if let Some(body) = body {
            request = request.json(&body);
        }
        // No retry. Dropping this future cancels network work; it never resends a POST.
        let mut response = request.send().await.map_err(network_error)?;
        if !response.status().is_success() {
            return Err(status_map(response.status()));
        }
        if response.content_length().is_some_and(|n| n > cap as u64) {
            return Err(ApiError::Protocol);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if bytes.len().saturating_add(chunk.len()) > cap {
                return Err(ApiError::Protocol);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    pub async fn list(&self, host: &HostSnapshot) -> Result<SessionList, ApiError> {
        let bytes = self
            .request(host, Method::GET, "admin/sessions", None)
            .await?;
        parse_list(&bytes)
    }
    /// Official contract body is `{"name": name}` only; the terminal size is
    /// set with a resize control frame after attach.
    pub async fn create(&self, host: &HostSnapshot, name: &str) -> Result<SessionGrant, ApiError> {
        validate_session_name(name)?;
        let bytes = self
            .request(
                host,
                Method::POST,
                "admin/sessions",
                Some(serde_json::json!({"name":name})),
            )
            .await?;
        parse_grant(&bytes, name)
    }
    /// Official renew — explicit user-directed re-selection only, never
    /// driven by reconnect logic. Sends no `expected_*` fields: the
    /// runtime owns the generation bump that invalidates old tokens.
    pub async fn renew(&self, host: &HostSnapshot, name: &str) -> Result<SessionGrant, ApiError> {
        validate_session_name(name)?;
        let bytes = self
            .request(
                host,
                Method::POST,
                &format!("admin/sessions/{name}/renew"),
                None,
            )
            .await?;
        parse_grant(&bytes, name)
    }
    /// Explicit user-directed restart only. Never called by reconnect logic.
    pub async fn restart(&self, host: &HostSnapshot, name: &str) -> Result<SessionGrant, ApiError> {
        validate_session_name(name)?;
        let bytes = self
            .request(
                host,
                Method::POST,
                &format!("admin/sessions/{name}/restart"),
                None,
            )
            .await?;
        parse_grant(&bytes, name)
    }
    /// Explicit operator kill only; local tab close must not call this.
    pub async fn delete(&self, host: &HostSnapshot, name: &str) -> Result<(), ApiError> {
        validate_session_name(name)?;
        self.request(
            host,
            Method::DELETE,
            &format!("admin/sessions/{name}"),
            None,
        )
        .await?;
        Ok(())
    }
}
fn parse_list(bytes: &[u8]) -> Result<SessionList, ApiError> {
    let list: SessionList = serde_json::from_slice(bytes).map_err(|_| ApiError::Protocol)?;
    if list.sessions.len() > 128 {
        return Err(ApiError::Protocol);
    }
    let mut names = std::collections::HashSet::new();
    for s in &list.sessions {
        validate_session_name(&s.name)?;
        if s.generation == 0
            || !names.insert(&s.name)
            || s.tier.len() > 128
            || s.tier.chars().any(char::is_control)
        {
            return Err(ApiError::Protocol);
        }
    }
    Ok(list)
}
fn parse_grant(bytes: &[u8], expected: &str) -> Result<SessionGrant, ApiError> {
    let grant: WireGrant = serde_json::from_slice(bytes).map_err(|_| ApiError::Protocol)?;
    if grant.session != expected
        || grant.generation == 0
        || grant.token_expires_in_secs == 0
        || grant.token.expose().len() != 64
        || !grant.token.expose().bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError::Protocol);
    }
    Ok(SessionGrant {
        session: grant.session,
        generation: grant.generation,
        token: grant.token,
        token_expires_in_secs: grant.token_expires_in_secs,
    })
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    #[test]
    fn wire_metadata_and_grants_validate_without_token_readback() {
        // Official reply carries no compatibility/runtime_instance/
        // session_identity fields; parse must not require them.
        let bytes = br#"{"sessions":[{"name":"fixture","generation":1,"alive":true,"attached":false,"bytes_written":28,"tier":"tier1-best-effort-process-group","teardown_best_effort":true,"absolute_ttl_best_effort":true}],"draining":false}"#;
        let list = parse_list(bytes).unwrap();
        assert!(list.sessions[0].teardown_best_effort);
        let safe = serde_json::to_value(&list).unwrap();
        assert_eq!(safe["sessions"][0]["bytesWritten"], 28);
        assert!(safe.get("token").is_none());
        let token = "a".repeat(64);
        let bytes=serde_json::to_vec(&serde_json::json!({"session":"fixture","generation":2,"token":token,"token_expires_in_secs":100})).unwrap();
        let grant = parse_grant(&bytes, "fixture").unwrap();
        assert_eq!(grant.generation, 2);
        assert!(!format!("{grant:?}").contains(&token));
        assert!(parse_grant(&bytes, "another").is_err());
        let mut bad = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
        bad["generation"] = serde_json::json!(0);
        assert!(parse_grant(&serde_json::to_vec(&bad).unwrap(), "fixture").is_err());
    }
    #[test]
    fn invalid_and_duplicate_remote_sessions_are_rejected() {
        let session = serde_json::json!({"name":"fixture","generation":1,"alive":true,"attached":false,"bytes_written":0,"tier":"tier1","teardown_best_effort":true,"absolute_ttl_best_effort":true});
        for sessions in [
            vec![session.clone(), session.clone()],
            vec![session.clone(); 129],
        ] {
            let bytes =
                serde_json::to_vec(&serde_json::json!({"sessions":sessions,"draining":false}))
                    .unwrap();
            assert!(parse_list(&bytes).is_err());
        }
        for name in ["", "../target", "BAD", "white space", "a?token=x"] {
            assert!(validate_session_name(name).is_err());
        }
    }
}
