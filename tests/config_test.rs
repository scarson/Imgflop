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
fn runtime_config_requires_admin_env() {
    let map = std::collections::HashMap::new();
    let err =
        imgflop::config::RuntimeConfig::from_map(&map).expect_err("missing admin env should fail");
    assert!(err.contains("ADMIN_USER"));
}

#[test]
fn runtime_config_parses_auth_and_polling_overrides() {
    let mut map = std::collections::HashMap::new();
    map.insert("ADMIN_USER".to_string(), "admin".to_string());
    map.insert(
        "ADMIN_PASSWORD_HASH".to_string(),
        "$argon2id$dummy".to_string(),
    );
    map.insert("IMGFLOP_HISTORY_TOP_N".to_string(), "250".to_string());
    map.insert("IMGFLOP_POLL_INTERVAL_SECS".to_string(), "30".to_string());
    map.insert("IMGFLOP_COOKIE_SECURE".to_string(), "true".to_string());
    map.insert("IMGFLOP_SESSION_TTL_SECS".to_string(), "7200".to_string());

    let cfg = imgflop::config::RuntimeConfig::from_map(&map).expect("runtime config should parse");
    assert_eq!(cfg.history_top_n, 250);
    assert_eq!(cfg.poll_interval_secs, 30);
    assert_eq!(cfg.auth.session_ttl_secs, 7200);
    assert!(cfg.auth.secure_cookie);
}

#[test]
fn runtime_config_parses_api_top_n_env() {
    let mut map = std::collections::HashMap::new();
    map.insert("ADMIN_USER".to_string(), "admin".to_string());
    map.insert(
        "ADMIN_PASSWORD_HASH".to_string(),
        "$argon2id$dummy".to_string(),
    );
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
    let mut map = std::collections::HashMap::new();
    map.insert("ADMIN_USER".to_string(), "admin".to_string());
    map.insert(
        "ADMIN_PASSWORD_HASH".to_string(),
        "$argon2id$dummy".to_string(),
    );
    map.insert("IMGFLOP_API_TOP_N".to_string(), "0".to_string());

    let err = imgflop::config::RuntimeConfig::from_map(&map)
        .expect_err("api top-n of 0 should be rejected");
    assert!(err.contains("IMGFLOP_API_TOP_N"));
}
