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
    admin_user: String,
    admin_password_hash: String,
    sessions: Mutex<HashSet<SessionRecord>>,
    session_seq: AtomicU64,
    session_ttl_secs: u64,
    secure_cookie: bool,
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
        if admin_user.trim().is_empty() {
            return Err("admin username must not be empty".to_string());
        }
        PasswordHash::new(&admin_password_hash)
            .map_err(|_| "admin password hash is invalid".to_string())?;
        if session_ttl_secs == 0 {
            return Err("session TTL must be >= 1 second".to_string());
        }

        Ok(Self {
            admin_user,
            admin_password_hash,
            sessions: Mutex::new(HashSet::new()),
            session_seq: AtomicU64::new(1),
            session_ttl_secs,
            secure_cookie,
        })
    }

    pub fn dev_default() -> Self {
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(b"admin", &salt)
            .expect("default dev password hash should build")
            .to_string();

        Self::new("admin".to_string(), password_hash, 3600, false)
            .expect("dev default auth should be valid")
    }

    pub fn login(&self, username: &str, password: &str) -> Result<String, AuthError> {
        if !self.verify_credentials(username, password) {
            return Err(AuthError::InvalidCredentials);
        }

        let token = format!("s-{}", self.session_seq.fetch_add(1, Ordering::Relaxed));
        let issued_at_utc = now_epoch_seconds();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(SessionRecord {
                token: token.clone(),
                issued_at_utc,
            });
        }
        Ok(token)
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

    fn verify_credentials(&self, username: &str, password: &str) -> bool {
        if username != self.admin_user {
            return false;
        }

        let Ok(parsed_hash) = PasswordHash::new(&self.admin_password_hash) else {
            return false;
        };

        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
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
