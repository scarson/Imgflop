use serde::Deserialize;

use crate::sources::MemeCandidate;

pub fn parse_memes(body: &str) -> Result<Vec<MemeCandidate>, String> {
    let payload: ImgflipApiResponse = serde_json::from_str(body).map_err(|err| err.to_string())?;

    if !payload.success {
        return Err("imgflip API returned success=false".to_string());
    }

    Ok(payload
        .data
        .memes
        .into_iter()
        .enumerate()
        .map(|(index, meme)| MemeCandidate {
            source_meme_id: meme.id.clone(),
            name: meme.name,
            image_url: meme.url,
            page_url: format!("https://imgflip.com/memegenerator/{}", meme.id),
            width: meme.width,
            height: meme.height,
            rank: (index + 1) as u32,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct ImgflipApiResponse {
    success: bool,
    data: ImgflipData,
}

#[derive(Debug, Deserialize)]
struct ImgflipData {
    memes: Vec<ImgflipMeme>,
}

#[derive(Debug, Deserialize)]
struct ImgflipMeme {
    id: String,
    name: String,
    url: String,
    width: u32,
    height: u32,
}
