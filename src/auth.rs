use std::{
    collections::HashSet,
    env,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::Context;
use axum::{
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use serde_json::json;
use subtle::ConstantTimeEq;

use crate::config::{AuthConfig, AuthMode};

#[derive(Clone)]
pub struct AuthLayer {
    config: AuthConfig,
    jwks: Option<JwkSet>,
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    exp: usize,
    #[serde(default)]
    nbf: Option<usize>,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    scp: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug)]
pub struct AuthError {
    status: StatusCode,
    message: String,
    www_authenticate: Option<String>,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let mut response = (self.status, self.message).into_response();
        if let Some(value) = self.www_authenticate
            && let Ok(value) = value.parse()
        {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        response
    }
}

impl AuthError {
    /// 429 response used by the request handler when a client is banned
    /// by [`AuthThrottle`] for repeated failures.
    pub fn banned(remaining: Duration) -> AuthError {
        AuthError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: format!(
                "too many failed auth attempts; retry in {} seconds",
                remaining.as_secs().max(1)
            ),
            www_authenticate: None,
        }
    }
}

impl AuthLayer {
    pub async fn new(config: AuthConfig) -> anyhow::Result<Self> {
        let jwks = if config.mode == AuthMode::OAuthJwks {
            Some(
                reqwest::get(&config.oauth.jwks_url)
                    .await
                    .context("fetch jwks")?
                    .json::<JwkSet>()
                    .await
                    .context("parse jwks")?,
            )
        } else {
            None
        };
        Ok(Self { config, jwks })
    }

    pub fn protected_resource_metadata(&self) -> serde_json::Value {
        json!({
            "resource": self.config.oauth.resource,
            "authorization_servers": [self.config.oauth.issuer],
            "jwks_uri": self.config.oauth.jwks_url,
            "scopes_supported": self.config.oauth.required_scopes,
            "bearer_methods_supported": ["header"],
        })
    }

    pub fn check(&self, headers: &HeaderMap) -> Result<(), AuthError> {
        match self.config.mode {
            AuthMode::None => Ok(()),
            AuthMode::StaticBearer => self.check_static(headers),
            AuthMode::FakeOAuth => self.check_static(headers),
            AuthMode::OAuthJwks => self.check_oauth(headers),
        }
    }

    fn bearer<'a>(&self, headers: &'a HeaderMap) -> Result<&'a str, AuthError> {
        let Some(value) = headers.get(header::AUTHORIZATION) else {
            return Err(self.challenge("missing bearer token"));
        };
        let value = value
            .to_str()
            .map_err(|_| self.challenge("invalid authorization header"))?;
        value
            .strip_prefix("Bearer ")
            .ok_or_else(|| self.challenge("missing bearer token"))
    }

    fn check_static(&self, headers: &HeaderMap) -> Result<(), AuthError> {
        let presented = self.bearer(headers)?;
        let expected = self.static_bearer_token()?;
        if constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
            Ok(())
        } else {
            Err(self.challenge("invalid bearer token"))
        }
    }

    pub fn static_bearer_token(&self) -> Result<String, AuthError> {
        match env::var(&self.config.static_bearer.token_env) {
            Ok(token) if !token.is_empty() => Ok(token),
            _ => self
                .config
                .static_bearer
                .token
                .clone()
                .filter(|token| !token.is_empty())
                .ok_or_else(|| {
                    self.server_error(
                        "static bearer token is not set; configure token or token_env",
                    )
                }),
        }
    }

    fn check_oauth(&self, headers: &HeaderMap) -> Result<(), AuthError> {
        let token = self.bearer(headers)?;
        let header = decode_header(token).map_err(|_| self.challenge("invalid jwt header"))?;
        let kid = header
            .kid
            .ok_or_else(|| self.challenge("jwt missing kid"))?;
        let jwks = self
            .jwks
            .as_ref()
            .ok_or_else(|| self.server_error("jwks unavailable"))?;
        let jwk = jwks
            .find(&kid)
            .ok_or_else(|| self.challenge("unknown jwt key id"))?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| self.server_error("invalid jwk"))?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(std::slice::from_ref(&self.config.oauth.issuer));
        validation.set_audience(std::slice::from_ref(&self.config.oauth.audience));
        let data = decode::<Claims>(token, &key, &validation)
            .map_err(|_| self.challenge("invalid jwt"))?;
        let claims = data.claims;
        if claims.iss != self.config.oauth.issuer {
            return Err(self.challenge("invalid issuer"));
        }
        if !aud_contains(&claims.aud, &self.config.oauth.audience) {
            return Err(self.challenge("invalid audience"));
        }
        let mut scopes: HashSet<String> = claims
            .scope
            .split_whitespace()
            .map(str::to_string)
            .collect();
        scopes.extend(claims.scp);
        if !self
            .config
            .oauth
            .required_scopes
            .iter()
            .all(|s| scopes.contains(s))
        {
            return Err(self.challenge("missing required scope"));
        }
        let _ = claims.exp;
        let _ = claims.nbf;
        Ok(())
    }

    fn challenge(&self, message: &str) -> AuthError {
        AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: message.to_string(),
            www_authenticate: Some(format!(
                r#"Bearer resource_metadata="{}""#,
                "/.well-known/oauth-protected-resource"
            )),
        }
    }

    fn server_error(&self, message: &str) -> AuthError {
        AuthError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.to_string(),
            www_authenticate: None,
        }
    }
}

fn aud_contains(aud: &Audience, expected: &str) -> bool {
    match aud {
        Audience::One(a) => a == expected,
        Audience::Many(v) => v.iter().any(|a| a == expected),
    }
}

/// Constant-time byte comparison for token equality checks.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// The real client IP behind the Cloudflare tunnel, when present.
///
/// The server binds to loopback, so the only sources are local processes
/// and cloudflared, which forwards the edge-set `CF-Connecting-IP` header.
/// Cloudflare strips any client-supplied value of this header at the edge.
pub fn client_ip(headers: &HeaderMap, peer: IpAddr) -> IpAddr {
    headers
        .get("cf-connecting-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(peer)
}

/// Tracks failed bearer authentications per client IP and applies an
/// exponentially growing ban, so token spraying over the public URL is
/// both throttled and visible in logs.
pub struct AuthThrottle {
    inner: Mutex<std::collections::HashMap<IpAddr, Offender>>,
    /// Failures tolerated before the first ban.
    threshold: u32,
    /// Duration of the first ban; doubles on every further failure.
    base_ban: Duration,
    /// Upper bound for a ban.
    max_ban: Duration,
    /// Defensive cap on tracked IPs (memory-exhaustion guard).
    max_entries: usize,
}

#[derive(Debug)]
struct Offender {
    failures: u32,
    banned_until: Option<Instant>,
}

impl AuthThrottle {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(std::collections::HashMap::new()),
            threshold: 3,
            base_ban: Duration::from_secs(2),
            max_ban: Duration::from_secs(3600),
            max_entries: 10_000,
        }
    }

    /// Ok(()) when the IP may attempt authentication. Err(remaining) while banned.
    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        let now = Instant::now();
        let offenders = self.inner.lock().expect("auth throttle poisoned");
        if let Some(offender) = offenders.get(&ip)
            && let Some(until) = offender.banned_until
            && until > now
        {
            return Err(until - now);
        }
        Ok(())
    }

    /// Records a failed attempt and returns the ban just entered, if any.
    pub fn record_failure(&self, ip: IpAddr) -> Option<Duration> {
        let now = Instant::now();
        let mut offenders = self.inner.lock().expect("auth throttle poisoned");
        if offenders.len() >= self.max_entries && !offenders.contains_key(&ip) {
            return None; // table full; fail open rather than blocking new faces
        }
        let offender = offenders.entry(ip).or_insert(Offender {
            failures: 0,
            banned_until: None,
        });
        offender.failures += 1;
        if offender.failures < self.threshold {
            return None;
        }
        let exponent = offender.failures - self.threshold;
        let ban = self
            .base_ban
            .checked_mul(1_u32 << exponent)
            .unwrap_or(self.max_ban)
            .min(self.max_ban);
        offender.banned_until = Some(now + ban);
        Some(ban)
    }

    /// A successful authentication clears the IP's record entirely.
    pub fn record_success(&self, ip: IpAddr) {
        self.inner
            .lock()
            .expect("auth throttle poisoned")
            .remove(&ip);
    }
}

impl Default for AuthThrottle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_bearer_accepts_and_rejects() {
        unsafe { env::set_var("agentbox_TEST_TOKEN", "secret") };
        let mut config = AuthConfig::default();
        config.static_bearer.token_env = "agentbox_TEST_TOKEN".to_string();
        let auth = AuthLayer { config, jwks: None };
        let mut headers = HeaderMap::new();
        assert!(auth.check(&headers).is_err());
        headers.insert(header::AUTHORIZATION, "Bearer nope".parse().unwrap());
        assert!(auth.check(&headers).is_err());
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(auth.check(&headers).is_ok());
    }

    #[test]
    fn static_bearer_accepts_config_token() {
        let mut config = AuthConfig::default();
        config.static_bearer.token_env = "agentbox_TEST_MISSING_TOKEN".to_string();
        config.static_bearer.token = Some("config-secret".to_string());
        let auth = AuthLayer { config, jwks: None };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer config-secret".parse().unwrap(),
        );
        assert!(auth.check(&headers).is_ok());
    }

    #[test]
    fn throttle_bans_after_threshold_and_doubles() {
        let throttle = AuthThrottle::new();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        assert!(throttle.check(ip).is_ok());
        assert_eq!(throttle.record_failure(ip), None); // failure 1
        assert_eq!(throttle.record_failure(ip), None); // failure 2
        assert_eq!(throttle.record_failure(ip), Some(Duration::from_secs(2))); // first ban
        assert!(throttle.check(ip).is_err()); // banned right away
        assert_eq!(throttle.record_failure(ip), Some(Duration::from_secs(4)));
        assert_eq!(throttle.record_failure(ip), Some(Duration::from_secs(8)));
        // capped at max_ban even after many failures
        for _ in 0..20 {
            throttle.record_failure(ip);
        }
        assert_eq!(throttle.record_failure(ip), Some(Duration::from_secs(3600)));
    }

    #[test]
    fn throttle_success_resets_and_other_ips_unaffected() {
        let throttle = AuthThrottle::new();
        let bad: IpAddr = "203.0.113.7".parse().unwrap();
        let other: IpAddr = "198.51.100.9".parse().unwrap();
        for _ in 0..3 {
            throttle.record_failure(bad);
        }
        assert!(throttle.check(bad).is_err());
        assert!(throttle.check(other).is_ok());
        throttle.record_success(bad);
        assert!(throttle.check(bad).is_ok());
    }

    #[test]
    fn client_ip_prefers_cf_header() {
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        assert_eq!(client_ip(&headers, peer), peer);
        headers.insert("cf-connecting-ip", "203.0.113.7".parse().unwrap());
        assert_eq!(client_ip(&headers, peer), ip);
        headers.insert("cf-connecting-ip", "not-an-ip".parse().unwrap());
        assert_eq!(client_ip(&headers, peer), peer);
    }
}
