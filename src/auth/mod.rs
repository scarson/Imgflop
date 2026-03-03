use std::{
    collections::HashSet,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::http::HeaderMap;
use rand_core::OsRng;

pub mod session;

#[derive(Debug)]
pub enum AuthError {
    InvalidCredentials,
}

pub struct AuthService {
    fallback_admin: Option<FallbackAdmin>,
    sessions: Mutex<HashSet<SessionRecord>>,
    session_seq: AtomicU64,
    session_ttl_secs: u64,
    secure_cookie: bool,
}

#[derive(Debug, Clone)]
struct FallbackAdmin {
    username: String,
    password_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionRecord {
    token: String,
    issued_at_utc: i64,
}

impl AuthService {
    pub fn new(
        admin_user: String,
        admin_password_hash: String,
        session_ttl_secs: u64,
        secure_cookie: bool,
    ) -> Result<Self, String> {
        Self::new_with_fallback(
            Some(admin_user),
            Some(admin_password_hash),
            session_ttl_secs,
            secure_cookie,
        )
    }

    pub fn new_with_fallback(
        admin_user: Option<String>,
        admin_password_hash: Option<String>,
        session_ttl_secs: u64,
        secure_cookie: bool,
    ) -> Result<Self, String> {
        if session_ttl_secs == 0 {
            return Err("session TTL must be >= 1 second".to_string());
        }

        let fallback_admin = match (admin_user, admin_password_hash) {
            (Some(username), Some(password_hash)) => {
                if username.trim().is_empty() {
                    return Err("admin username must not be empty".to_string());
                }
                PasswordHash::new(&password_hash)
                    .map_err(|_| "admin password hash is invalid".to_string())?;
                Some(FallbackAdmin {
                    username,
                    password_hash,
                })
            }
            (None, None) => None,
            _ => return Err("ADMIN_USER and ADMIN_PASSWORD_HASH must both be set".to_string()),
        };

        Ok(Self {
            fallback_admin,
            sessions: Mutex::new(HashSet::new()),
            session_seq: AtomicU64::new(1),
            session_ttl_secs,
            secure_cookie,
        })
    }

    pub fn dev_default() -> Self {
        let password_hash = hash_password("admin").expect("default dev password hash should build");

        Self::new_with_fallback(Some("admin".to_string()), Some(password_hash), 3600, false)
            .expect("dev default auth should be valid")
    }

    pub fn login(&self, username: &str, password: &str) -> Result<String, AuthError> {
        if !self.verify_fallback_credentials(username, password) {
            return Err(AuthError::InvalidCredentials);
        }

        Ok(self.issue_session_token())
    }

    pub fn logout_token(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|session| session.token != token);
        }
    }

    pub fn is_authenticated_headers(&self, headers: &HeaderMap) -> bool {
        let Some(token) = session::extract_session_token(headers) else {
            return false;
        };

        let now = now_epoch_seconds();
        self.sessions
            .lock()
            .map(|mut sessions| {
                sessions.retain(|entry| !self.is_expired(now, entry.issued_at_utc));
                sessions.iter().any(|entry| entry.token == token)
            })
            .unwrap_or(false)
    }

    pub fn session_ttl_secs(&self) -> u64 {
        self.session_ttl_secs
    }

    pub fn secure_cookie(&self) -> bool {
        self.secure_cookie
    }

    pub fn has_fallback_credentials(&self) -> bool {
        self.fallback_admin.is_some()
    }

    pub fn issue_session_token(&self) -> String {
        let token = format!("s-{}", self.session_seq.fetch_add(1, Ordering::Relaxed));
        let issued_at_utc = now_epoch_seconds();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(SessionRecord {
                token: token.clone(),
                issued_at_utc,
            });
        }
        token
    }

    pub fn verify_fallback_credentials(&self, username: &str, password: &str) -> bool {
        let Some(fallback) = self.fallback_admin.as_ref() else {
            return false;
        };
        if username != fallback.username {
            return false;
        }
        verify_password(&fallback.password_hash, password)
    }

    fn is_expired(&self, now_utc: i64, issued_at_utc: i64) -> bool {
        now_utc.saturating_sub(issued_at_utc) >= self.session_ttl_secs as i64
    }
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub fn hash_password(password: &str) -> Result<String, String> {
    if password.is_empty() {
        return Err("password must not be empty".to_string());
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| err.to_string())
}

pub fn verify_password(password_hash: &str, password: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}
