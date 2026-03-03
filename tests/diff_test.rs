fn state(meme_id: &str, rank: u32) -> imgflop::diff::RankedState {
    imgflop::diff::RankedState {
        meme_id: meme_id.to_string(),
        rank,
        metadata_hash: None,
    }
}

#[test]
fn unchanged_rank_emits_no_events() {
    let prev = vec![state("m1", 1)];
    let next = vec![state("m1", 1)];
    let events = imgflop::diff::compute(&prev, &next);

    assert!(events.is_empty());
}
