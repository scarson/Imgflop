pub mod api;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemeCandidate {
    pub source_meme_id: String,
    pub name: String,
    pub image_url: String,
    pub page_url: String,
    pub width: u32,
    pub height: u32,
    pub rank: u32,
}
