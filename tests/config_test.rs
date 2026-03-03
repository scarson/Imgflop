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
