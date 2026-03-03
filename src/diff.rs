use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedState {
    pub meme_id: String,
    pub rank: u32,
    pub metadata_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffEvent {
    EnteredTop {
        meme_id: String,
        new_rank: u32,
    },
    LeftTop {
        meme_id: String,
        old_rank: u32,
    },
    RankChanged {
        meme_id: String,
        old_rank: u32,
        new_rank: u32,
    },
    MetadataChanged {
        meme_id: String,
    },
}

pub fn compute(prev: &[RankedState], next: &[RankedState]) -> Vec<DiffEvent> {
    let prev_map: HashMap<&str, &RankedState> = prev.iter().map(|row| (row.meme_id.as_str(), row)).collect();
    let next_map: HashMap<&str, &RankedState> = next.iter().map(|row| (row.meme_id.as_str(), row)).collect();

    let mut events = Vec::new();

    for row in next {
        match prev_map.get(row.meme_id.as_str()) {
            None => events.push(DiffEvent::EnteredTop {
                meme_id: row.meme_id.clone(),
                new_rank: row.rank,
            }),
            Some(previous) => {
                if previous.rank != row.rank {
                    events.push(DiffEvent::RankChanged {
                        meme_id: row.meme_id.clone(),
                        old_rank: previous.rank,
                        new_rank: row.rank,
                    });
                }
                if previous.metadata_hash != row.metadata_hash {
                    events.push(DiffEvent::MetadataChanged {
                        meme_id: row.meme_id.clone(),
                    });
                }
            }
        }
    }

    for row in prev {
        if !next_map.contains_key(row.meme_id.as_str()) {
            events.push(DiffEvent::LeftTop {
                meme_id: row.meme_id.clone(),
                old_rank: row.rank,
            });
        }
    }

    events
}
