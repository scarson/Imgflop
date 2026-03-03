use std::{collections::HashMap, env};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiTopN {
    Max,
    Int(u32),
}

impl ApiTopN {
    pub fn is_max(&self) -> bool {
        matches!(self, Self::Max)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingConfig {
    pub api_top_n: ApiTopN,
    pub history_top_n: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub polling: PollingConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAuthConfig {
    pub admin_user: String,
    pub admin_password_hash: String,
    pub session_ttl_secs: u64,
    pub secure_cookie: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub bind: String,
    pub database_url: String,
    pub assets_dir: String,
    pub api_top_n: ApiTopN,
    pub history_top_n: u32,
    pub poll_interval_secs: u64,
    pub api_endpoint: Option<String>,
    pub auth: RuntimeAuthConfig,
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_map(&env::vars().collect())
    }

    pub fn from_map(values: &HashMap<String, String>) -> Result<Self, String> {
        let get = |key: &str| values.get(key).cloned();

        let bind = get("IMGFLOP_BIND").unwrap_or_else(|| "127.0.0.1:8080".to_string());
        let database_url =
            get("IMGFLOP_DB_URL").unwrap_or_else(|| "sqlite://imgflop.db?mode=rwc".to_string());
        let assets_dir = get("IMGFLOP_ASSETS_DIR").unwrap_or_else(|| "data/images".to_string());
        let api_top_n = parse_api_top_n_env(get("IMGFLOP_API_TOP_N"))?;

        let history_top_n =
            parse_u32_at_least_one(get("IMGFLOP_HISTORY_TOP_N"), "IMGFLOP_HISTORY_TOP_N", 100)?;
        let poll_interval_secs = parse_u64_at_least_one(
            get("IMGFLOP_POLL_INTERVAL_SECS"),
            "IMGFLOP_POLL_INTERVAL_SECS",
            300,
        )?;

        let admin_user = get("ADMIN_USER")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "ADMIN_USER must be set".to_string())?;
        let admin_password_hash = get("ADMIN_PASSWORD_HASH")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "ADMIN_PASSWORD_HASH must be set".to_string())?;

        let session_ttl_secs = parse_u64_at_least_one(
            get("IMGFLOP_SESSION_TTL_SECS"),
            "IMGFLOP_SESSION_TTL_SECS",
            3600,
        )?;
        let secure_cookie = parse_bool(get("IMGFLOP_COOKIE_SECURE"), false);
        let api_endpoint = get("IMGFLOP_API_ENDPOINT").filter(|value| !value.trim().is_empty());

        Ok(Self {
            bind,
            database_url,
            assets_dir,
            api_top_n,
            history_top_n,
            poll_interval_secs,
            api_endpoint,
            auth: RuntimeAuthConfig {
                admin_user,
                admin_password_hash,
                session_ttl_secs,
                secure_cookie,
            },
        })
    }
}

pub fn from_toml(input: &str) -> Result<Config, String> {
    let raw: RawConfig = toml::from_str(input).map_err(|err| err.to_string())?;
    let api_top_n = match raw.polling.api_top_n {
        RawApiTopN::Int(value) if value >= 1 => ApiTopN::Int(value),
        RawApiTopN::Int(_) => return Err("polling.api_top_n must be >= 1".to_string()),
        RawApiTopN::String(value) => {
            if value.eq_ignore_ascii_case("max") {
                ApiTopN::Max
            } else {
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| "polling.api_top_n must be 'max' or integer >= 1".to_string())?;
                if parsed == 0 {
                    return Err("polling.api_top_n must be >= 1".to_string());
                }
                ApiTopN::Int(parsed)
            }
        }
    };

    if raw.polling.history_top_n == 0 {
        return Err("polling.history_top_n must be >= 1".to_string());
    }

    Ok(Config {
        polling: PollingConfig {
            api_top_n,
            history_top_n: raw.polling.history_top_n,
        },
    })
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    polling: RawPollingConfig,
}

#[derive(Debug, Deserialize)]
struct RawPollingConfig {
    api_top_n: RawApiTopN,
    history_top_n: u32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawApiTopN {
    String(String),
    Int(u32),
}

fn parse_u32_at_least_one(value: Option<String>, key: &str, default: u32) -> Result<u32, String> {
    match value {
        Some(raw) => {
            let parsed = raw
                .parse::<u32>()
                .map_err(|_| format!("{key} must be an integer >= 1"))?;
            if parsed == 0 {
                return Err(format!("{key} must be >= 1"));
            }
            Ok(parsed)
        }
        None => Ok(default),
    }
}

fn parse_u64_at_least_one(value: Option<String>, key: &str, default: u64) -> Result<u64, String> {
    match value {
        Some(raw) => {
            let parsed = raw
                .parse::<u64>()
                .map_err(|_| format!("{key} must be an integer >= 1"))?;
            if parsed == 0 {
                return Err(format!("{key} must be >= 1"));
            }
            Ok(parsed)
        }
        None => Ok(default),
    }
}

fn parse_bool(value: Option<String>, default: bool) -> bool {
    match value {
        Some(raw) => {
            let lowered = raw.trim().to_ascii_lowercase();
            lowered == "1" || lowered == "true" || lowered == "yes" || lowered == "on"
        }
        None => default,
    }
}

fn parse_api_top_n_env(value: Option<String>) -> Result<ApiTopN, String> {
    let Some(raw) = value else {
        return Ok(ApiTopN::Max);
    };

    if raw.eq_ignore_ascii_case("max") {
        return Ok(ApiTopN::Max);
    }

    let parsed = raw
        .parse::<u32>()
        .map_err(|_| "IMGFLOP_API_TOP_N must be 'max' or integer >= 1".to_string())?;
    if parsed == 0 {
        return Err("IMGFLOP_API_TOP_N must be >= 1".to_string());
    }

    Ok(ApiTopN::Int(parsed))
}
