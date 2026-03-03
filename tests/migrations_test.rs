#[tokio::test]
async fn migrations_create_core_tables() {
    let pool = imgflop::store::db::test_pool().await;
    let tables = imgflop::store::db::table_names(&pool)
        .await
        .expect("table query should work");

    assert!(tables.contains(&"poll_runs".to_string()));
    assert!(tables.contains(&"top_state_events".to_string()));
}
