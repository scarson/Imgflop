use std::collections::HashMap;

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};

#[test]
fn parses_api_max_and_history_top_n() {
    let cfg = imgflop::config::from_toml(
        r#"
[polling]
api_top_n = "max"
history_top_n = 2000
"#,
    )
    .expect("config should parse");

    assert!(cfg.polling.api_top_n.is_max());
    assert_eq!(cfg.polling.history_top_n, 2000);
}

#[test]
fn runtime_config_allows_missing_admin_env() {
    let map = HashMap::new();
    let cfg = imgflop::config::RuntimeConfig::from_map(&map)
        .expect("missing admin env should allow db-backed setup");
    assert!(cfg.auth.fallback_admin_user.is_none());
    assert!(cfg.auth.fallback_admin_password_hash.is_none());
}

#[test]
fn runtime_config_rejects_partial_admin_fallback_env() {
    let mut map = HashMap::new();
    map.insert("ADMIN_USER".to_string(), "admin".to_string());
    let err = imgflop::config::RuntimeConfig::from_map(&map)
        .expect_err("partial admin fallback env should fail");
    assert!(err.contains("must both be set"));
}

#[test]
fn runtime_config_parses_auth_and_polling_overrides() {
    let mut map = base_runtime_map();
    map.insert("IMGFLOP_HISTORY_TOP_N".to_string(), "250".to_string());
    map.insert("IMGFLOP_POLL_INTERVAL_SECS".to_string(), "30".to_string());
    map.insert("IMGFLOP_COOKIE_SECURE".to_string(), "true".to_string());
    map.insert("IMGFLOP_SESSION_TTL_SECS".to_string(), "7200".to_string());

    let cfg = imgflop::config::RuntimeConfig::from_map(&map).expect("runtime config should parse");
    assert_eq!(cfg.history_top_n, 250);
    assert_eq!(cfg.poll_interval_secs, 30);
    assert_eq!(cfg.auth.session_ttl_secs, 7200);
    assert!(cfg.auth.secure_cookie);
    assert_eq!(cfg.auth.fallback_admin_user.as_deref(), Some("admin"));
    assert!(cfg.auth.fallback_admin_password_hash.is_some());
}

#[test]
fn runtime_config_parses_api_top_n_env() {
    let mut map = base_runtime_map();
    map.insert("IMGFLOP_API_TOP_N".to_string(), "25".to_string());

    let cfg = imgflop::config::RuntimeConfig::from_map(&map).expect("runtime config should parse");
    assert_eq!(cfg.api_top_n, imgflop::config::ApiTopN::Int(25));

    map.insert("IMGFLOP_API_TOP_N".to_string(), "max".to_string());
    let max_cfg =
        imgflop::config::RuntimeConfig::from_map(&map).expect("max api top-n should parse");
    assert!(max_cfg.api_top_n.is_max());
}

#[test]
fn runtime_config_rejects_invalid_api_top_n_env() {
    let mut map = base_runtime_map();
    map.insert("IMGFLOP_API_TOP_N".to_string(), "0".to_string());

    let err = imgflop::config::RuntimeConfig::from_map(&map)
        .expect_err("api top-n of 0 should be rejected");
    assert!(err.contains("IMGFLOP_API_TOP_N"));
}

#[test]
fn runtime_config_rejects_invalid_bind() {
    let mut map = base_runtime_map();
    map.insert("IMGFLOP_BIND".to_string(), "not-a-socket".to_string());

    let err = imgflop::config::RuntimeConfig::from_map(&map)
        .expect_err("invalid bind address should fail");
    assert!(err.contains("IMGFLOP_BIND"));
}

#[test]
fn runtime_config_rejects_invalid_api_endpoint() {
    let mut map = base_runtime_map();
    map.insert("IMGFLOP_API_ENDPOINT".to_string(), "not a url".to_string());

    let err = imgflop::config::RuntimeConfig::from_map(&map)
        .expect_err("invalid api endpoint should fail");
    assert!(err.contains("IMGFLOP_API_ENDPOINT"));
}

#[test]
fn runtime_config_validate_startup_accepts_creatable_assets_dir() {
    let temp = tempfile::TempDir::new().expect("temp dir should create");
    let assets = temp.path().join("images");

    let mut map = base_runtime_map();
    map.insert(
        "IMGFLOP_ASSETS_DIR".to_string(),
        assets.to_string_lossy().to_string(),
    );

    let cfg = imgflop::config::RuntimeConfig::from_map(&map).expect("config should parse");
    cfg.validate_startup()
        .expect("startup validation should succeed");
    assert!(assets.is_dir());
}

#[test]
fn runtime_config_validate_startup_rejects_assets_file_path() {
    let temp = tempfile::TempDir::new().expect("temp dir should create");
    let file_path = temp.path().join("not-a-dir");
    std::fs::write(&file_path, b"file").expect("file should write");

    let mut map = base_runtime_map();
    map.insert(
        "IMGFLOP_ASSETS_DIR".to_string(),
        file_path.to_string_lossy().to_string(),
    );

    let cfg = imgflop::config::RuntimeConfig::from_map(&map).expect("config should parse");
    let err = cfg
        .validate_startup()
        .expect_err("startup validation should reject file path");
    assert!(err.contains("IMGFLOP_ASSETS_DIR"));
}

#[test]
fn runtime_config_validate_startup_rejects_invalid_db_url() {
    let mut map = base_runtime_map();
    map.insert("IMGFLOP_DB_URL".to_string(), "postgres://bad".to_string());

    let cfg = imgflop::config::RuntimeConfig::from_map(&map).expect("config should parse");
    let err = cfg
        .validate_startup()
        .expect_err("startup validation should reject invalid db url");
    assert!(err.contains("IMGFLOP_DB_URL"));
}

fn base_runtime_map() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let salt = SaltString::encode_b64(b"fixedsaltfixed12").expect("test salt should encode");
    let password_hash = Argon2::default()
        .hash_password(b"admin", &salt)
        .expect("password hash should build")
        .to_string();
    map.insert("ADMIN_USER".to_string(), "admin".to_string());
    map.insert("ADMIN_PASSWORD_HASH".to_string(), password_hash);
    map
}
