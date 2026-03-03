use axum::http::HeaderMap;

pub fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;

    cookie_header.split(';').map(str::trim).find_map(|entry| {
        entry
            .strip_prefix("imgflop_session=")
            .map(ToString::to_string)
    })
}
