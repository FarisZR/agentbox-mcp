use std::{collections::HashSet, env};

use anyhow::Context;
use axum::{
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use serde_json::json;

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

    fn static_bearer_token(&self) -> Result<String, AuthError> {
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

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..max {
        let aa = a.get(i).copied().unwrap_or(0);
        let bb = b.get(i).copied().unwrap_or(0);
        diff |= (aa ^ bb) as usize;
    }
    diff == 0
}

pub fn unsigned_test_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
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
}
