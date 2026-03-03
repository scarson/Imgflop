#[tokio::test]
async fn parses_imgflip_api_memes() {
    let body = include_str!("fixtures/imgflip_get_memes.json");
    let list = imgflop::sources::api::parse_memes(body).expect("parser should succeed");

    assert!(!list.is_empty());
    assert_eq!(list[0].source_meme_id, "181913649");
    assert_eq!(list[0].rank, 1);
}
