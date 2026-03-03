use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
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
    sessions: Mutex<HashSet<String>>,
    session_seq: AtomicU64,
}

impl AuthService {
    pub fn dev_default() -> Self {
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(b"admin", &salt)
            .expect("default dev password hash should build")
            .to_string();

        Self {
            admin_user: "admin".to_string(),
            admin_password_hash: password_hash,
            sessions: Mutex::new(HashSet::new()),
            session_seq: AtomicU64::new(1),
        }
    }

    pub fn login(&self, username: &str, password: &str) -> Result<String, AuthError> {
        if !self.verify_credentials(username, password) {
            return Err(AuthError::InvalidCredentials);
        }

        let token = format!("s-{}", self.session_seq.fetch_add(1, Ordering::Relaxed));
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(token.clone());
        }
        Ok(token)
    }

    pub fn logout_token(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(token);
        }
    }

    pub fn is_authenticated_headers(&self, headers: &HeaderMap) -> bool {
        let Some(token) = session::extract_session_token(headers) else {
            return false;
        };

        self.sessions
            .lock()
            .map(|sessions| sessions.contains(token.as_str()))
            .unwrap_or(false)
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
}
