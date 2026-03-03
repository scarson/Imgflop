use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    auth::{AuthService, session::extract_session_token},
    designer::{DesignerService, render::TextLayer},
    ops::{
        polling::{PollRuntime, trigger_and_spawn},
        scheduler::Scheduler,
    },
};

pub mod routes;

#[derive(Clone)]
struct AppState {
    scheduler: Arc<Scheduler>,
    poll_runtime: Option<Arc<PollRuntime>>,
    auth: Arc<AuthService>,
    pool: Option<SqlitePool>,
    designer: Option<DesignerService>,
}

pub fn app_router() -> Router {
    let scheduler = Arc::new(Scheduler::new());
    let auth = Arc::new(AuthService::dev_default());
    app_router_with_state(AppState {
        scheduler,
        poll_runtime: None,
        auth,
        pool: None,
        designer: None,
    })
}

pub fn app_router_with_scheduler(scheduler: Arc<Scheduler>) -> Router {
    app_router_with_scheduler_and_poll_runtime(scheduler, None)
}

pub fn app_router_with_scheduler_and_poll_runtime(
    scheduler: Arc<Scheduler>,
    poll_runtime: Option<Arc<PollRuntime>>,
) -> Router {
    let auth = Arc::new(AuthService::dev_default());
    app_router_with_state(AppState {
        scheduler,
        poll_runtime,
        auth,
        pool: None,
        designer: None,
    })
}

pub fn app_router_runtime(
    scheduler: Arc<Scheduler>,
    poll_runtime: Arc<PollRuntime>,
    auth: Arc<AuthService>,
    pool: SqlitePool,
    designer: DesignerService,
) -> Router {
    app_router_with_state(AppState {
        scheduler,
        poll_runtime: Some(poll_runtime),
        auth,
        pool: Some(pool),
        designer: Some(designer),
    })
}

fn app_router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/", get(gallery_page))
        .route("/memes/{id}", get(meme_detail_page))
        .route("/create", get(create_page))
        .route("/create/export", post(create_export))
        .route("/media/image/{id}", get(media_image))
        .route("/health", get(|| async { "ok" }))
        .route("/static/app.css", get(stylesheet))
        .route("/admin", get(admin_home))
        .route("/admin/login", post(admin_login))
        .route("/admin/logout", post(admin_logout))
        .route("/admin/poll", post(trigger_manual_poll))
        .with_state(state)
}

async fn gallery_page(State(state): State<AppState>) -> Html<String> {
    if let Some(pool) = state.pool.as_ref() {
        match load_gallery_rows(pool).await {
            Ok(rows) => return Html(render_gallery_html(&rows)),
            Err(err) => tracing::error!(error = %err, "failed to load gallery data"),
        }
    }
    Html(routes::gallery::render().to_string())
}

async fn meme_detail_page(State(state): State<AppState>, Path(meme_id): Path<i64>) -> Html<String> {
    if let Some(pool) = state.pool.as_ref() {
        match load_meme_detail(pool, meme_id).await {
            Ok(Some(detail)) => return Html(render_meme_detail_html(&detail)),
            Ok(None) => return Html(render_missing_detail_html(meme_id)),
            Err(err) => tracing::error!(error = %err, meme_id, "failed to load meme detail"),
        }
    }

    Html(routes::gallery::render().to_string())
}

async fn create_page(State(state): State<AppState>) -> Html<String> {
    if let Some(pool) = state.pool.as_ref() {
        match load_create_templates(pool).await {
            Ok(rows) => return Html(render_create_html(&rows)),
            Err(err) => tracing::error!(error = %err, "failed to load create templates"),
        }
    }

    Html(routes::create::render().to_string())
}

async fn create_export(State(state): State<AppState>, body: Bytes) -> StatusCode {
    let parsed = if body.is_empty() {
        CreateExportRequest::default()
    } else {
        serde_json::from_slice::<CreateExportRequest>(&body).unwrap_or_default()
    };

    if let Some(designer) = state.designer.as_ref() {
        let layers: Vec<TextLayer> = parsed
            .layers
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect();
        if let Err(err) = designer
            .export_from_template(parsed.base_meme_id, parsed.store.unwrap_or(true), &layers)
            .await
        {
            tracing::error!(error = %err, "create export failed");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }

    StatusCode::ACCEPTED
}

async fn media_image(
    State(state): State<AppState>,
    Path(asset_id): Path<i64>,
) -> impl IntoResponse {
    let Some(pool) = state.pool.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT disk_path, mime FROM image_assets WHERE id = ?",
    )
    .bind(asset_id)
    .fetch_optional(pool)
    .await;
    let Some((disk_path, mime)) = row.ok().flatten() else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match std::fs::read(&disk_path) {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn stylesheet() -> ([(&'static str, &'static str); 1], &'static str) {
    (
        [("content-type", "text/css; charset=utf-8")],
        include_str!("static/app.css"),
    )
}

async fn admin_home(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if state.auth.is_authenticated_headers(&headers) {
        if let Some(pool) = state.pool.as_ref() {
            match load_admin_rows(pool).await {
                Ok((runs, errors)) => Html(render_admin_html(&runs, &errors)).into_response(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to load admin rows");
                    Html(routes::admin::render()).into_response()
                }
            }
        } else {
            Html(routes::admin::render()).into_response()
        }
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn admin_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    match state.auth.login(&payload.username, &payload.password) {
        Ok(token) => {
            let mut cookie = format!(
                "imgflop_session={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
                state.auth.session_ttl_secs()
            );
            if state.auth.secure_cookie() {
                cookie.push_str("; Secure");
            }
            let mut response = StatusCode::NO_CONTENT.into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie).expect("cookie should be valid"),
            );
            response
        }
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn admin_logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = extract_session_token(&headers) {
        state.auth.logout_token(&token);
    }

    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("imgflop_session=; Max-Age=0; Path=/"),
    );
    response
}

async fn trigger_manual_poll(State(state): State<AppState>, headers: HeaderMap) -> StatusCode {
    if !state.auth.is_authenticated_headers(&headers) {
        return StatusCode::UNAUTHORIZED;
    }

    trigger_and_spawn(Arc::clone(&state.scheduler), state.poll_runtime.clone()).await;

    StatusCode::ACCEPTED
}

#[derive(Debug, Default, Deserialize)]
struct CreateExportRequest {
    store: Option<bool>,
    base_meme_id: Option<i64>,
    layers: Option<Vec<CreateLayer>>,
}

#[derive(Debug, Deserialize)]
struct CreateLayer {
    text: String,
    x: Option<u32>,
    y: Option<u32>,
    scale: Option<u32>,
    color_hex: Option<String>,
}

impl From<CreateLayer> for TextLayer {
    fn from(value: CreateLayer) -> Self {
        let color = value
            .color_hex
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or([255, 255, 255, 255]);
        Self {
            text: value.text,
            x: value.x.unwrap_or(24),
            y: value.y.unwrap_or(24),
            scale: value.scale.unwrap_or(4).max(1),
            color,
        }
    }
}

#[derive(Debug)]
struct GalleryRow {
    meme_id: i64,
    rank: i64,
    title: String,
    page_url: Option<String>,
    image_asset_id: Option<i64>,
}

#[derive(Debug)]
struct MemeEventRow {
    event_type: String,
    old_rank: Option<i64>,
    new_rank: Option<i64>,
    at_utc: String,
}

#[derive(Debug)]
struct MemeDetailView {
    meme_id: i64,
    title: String,
    page_url: Option<String>,
    image_asset_id: Option<i64>,
    events: Vec<MemeEventRow>,
}

#[derive(Debug)]
struct AdminRunRow {
    id: i64,
    status: String,
    started_at_utc: String,
    completed_at_utc: Option<String>,
}

#[derive(Debug)]
struct AdminErrorRow {
    run_id: i64,
    error_kind: String,
    message: String,
    at_utc: String,
}

#[derive(Debug)]
struct CreateTemplateRow {
    meme_id: i64,
    rank: i64,
    title: String,
    image_asset_id: Option<i64>,
}

async fn load_gallery_rows(pool: &SqlitePool) -> Result<Vec<GalleryRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, i64, String, Option<String>, Option<i64>)>(
        r#"
        SELECT t.meme_id, t.rank, m.title, m.page_url, m.image_asset_id
        FROM top_state_current t
        JOIN memes m ON m.id = t.meme_id
        WHERE t.scope = 'api'
        ORDER BY t.rank ASC
        LIMIT 200
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(meme_id, rank, title, page_url, image_asset_id)| GalleryRow {
                meme_id,
                rank,
                title,
                page_url,
                image_asset_id,
            },
        )
        .collect())
}

async fn load_meme_detail(
    pool: &SqlitePool,
    meme_id: i64,
) -> Result<Option<MemeDetailView>, sqlx::Error> {
    let meme = sqlx::query_as::<_, (String, Option<String>, Option<i64>)>(
        "SELECT title, page_url, image_asset_id FROM memes WHERE id = ?",
    )
    .bind(meme_id)
    .fetch_optional(pool)
    .await?;

    let Some((title, page_url, image_asset_id)) = meme else {
        return Ok(None);
    };

    let events = sqlx::query_as::<_, (String, Option<i64>, Option<i64>, String)>(
        r#"
        SELECT event_type, old_rank, new_rank, at_utc
        FROM top_state_events
        WHERE meme_id = ?
        ORDER BY id DESC
        LIMIT 50
        "#,
    )
    .bind(meme_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(event_type, old_rank, new_rank, at_utc)| MemeEventRow {
        event_type,
        old_rank,
        new_rank,
        at_utc,
    })
    .collect();

    Ok(Some(MemeDetailView {
        meme_id,
        title,
        page_url,
        image_asset_id,
        events,
    }))
}

async fn load_admin_rows(
    pool: &SqlitePool,
) -> Result<(Vec<AdminRunRow>, Vec<AdminErrorRow>), sqlx::Error> {
    let runs = sqlx::query_as::<_, (i64, String, String, Option<String>)>(
        r#"
        SELECT id, status, started_at_utc, completed_at_utc
        FROM poll_runs
        ORDER BY id DESC
        LIMIT 20
        "#,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(
        |(id, status, started_at_utc, completed_at_utc)| AdminRunRow {
            id,
            status,
            started_at_utc,
            completed_at_utc,
        },
    )
    .collect();

    let errors = sqlx::query_as::<_, (i64, String, String, String)>(
        r#"
        SELECT run_id, error_kind, message, at_utc
        FROM poll_run_errors
        ORDER BY id DESC
        LIMIT 20
        "#,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(run_id, error_kind, message, at_utc)| AdminErrorRow {
        run_id,
        error_kind,
        message,
        at_utc,
    })
    .collect();

    Ok((runs, errors))
}

async fn load_create_templates(pool: &SqlitePool) -> Result<Vec<CreateTemplateRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, i64, String, Option<i64>)>(
        r#"
        SELECT m.id, t.rank, m.title, m.image_asset_id
        FROM top_state_current t
        JOIN memes m ON m.id = t.meme_id
        WHERE t.scope = 'api'
        ORDER BY t.rank ASC
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(meme_id, rank, title, image_asset_id)| CreateTemplateRow {
            meme_id,
            rank,
            title,
            image_asset_id,
        })
        .collect())
}

fn render_gallery_html(rows: &[GalleryRow]) -> String {
    let mut html = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Top Memes</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><h1>Top Memes</h1><nav><a href="/create">Create</a> <a href="/admin">Admin</a></nav></header><main class="gallery">"#,
    );

    if rows.is_empty() {
        html.push_str(r#"<article class="card">No memes yet. Poll to ingest data.</article>"#);
    } else {
        for row in rows {
            let title = escape_html(&row.title);
            let link = format!("/memes/{}", row.meme_id);
            let source_link = row
                .page_url
                .as_deref()
                .map(escape_html)
                .unwrap_or_else(|| "#".to_string());
            let preview = row
                .image_asset_id
                .map(|asset_id| {
                    format!(
                        r#"<img class="thumb" src="/media/image/{asset_id}" alt="{title} preview" loading="lazy"/>"#,
                    )
                })
                .unwrap_or_else(|| r#"<div class="thumb placeholder">No Image</div>"#.to_string());
            html.push_str(&format!(
                r#"<article class="card">{preview}<h2>#{rank} <a href="{detail}">{title}</a></h2><p><a href="{source}" target="_blank" rel="noreferrer">Source</a></p></article>"#,
                preview = preview,
                rank = row.rank,
                detail = link,
                title = title,
                source = source_link
            ));
        }
    }

    html.push_str("</main></body></html>");
    html
}

fn render_meme_detail_html(view: &MemeDetailView) -> String {
    let mut html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>{title}</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><h1>{title}</h1><nav><a href="/">Gallery</a> <a href="/create">Create</a></nav></header><main><p>Meme ID: {id}</p>"#,
        title = escape_html(&view.title),
        id = view.meme_id
    );
    if let Some(asset_id) = view.image_asset_id {
        html.push_str(&format!(
            r#"<p><img class="detail-image" src="/media/image/{asset_id}" alt="{title} image"/></p>"#,
            title = escape_html(&view.title)
        ));
    }
    if let Some(page_url) = &view.page_url {
        html.push_str(&format!(
            r#"<p><a href="{url}" target="_blank" rel="noreferrer">Open Source Page</a></p>"#,
            url = escape_html(page_url)
        ));
    }
    html.push_str(r#"<h2>Recent Rank Events</h2><table><thead><tr><th>Type</th><th>Old</th><th>New</th><th>At</th></tr></thead><tbody>"#);
    if view.events.is_empty() {
        html.push_str(r#"<tr><td colspan="4">No events yet.</td></tr>"#);
    } else {
        for event in &view.events {
            html.push_str(&format!(
                r#"<tr><td>{event_type}</td><td>{old_rank}</td><td>{new_rank}</td><td>{at_utc}</td></tr>"#,
                event_type = escape_html(&event.event_type),
                old_rank = event
                    .old_rank
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                new_rank = event
                    .new_rank
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                at_utc = escape_html(&event.at_utc),
            ));
        }
    }
    html.push_str("</tbody></table></main></body></html>");
    html
}

fn render_missing_detail_html(meme_id: i64) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><title>Meme Not Found</title></head><body><main><h1>Meme Not Found</h1><p>No meme for id {meme_id}.</p></main></body></html>"#
    )
}

fn render_admin_html(runs: &[AdminRunRow], errors: &[AdminErrorRow]) -> String {
    let mut html = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Admin</title><link rel="stylesheet" href="/static/app.css"/></head><body><main><h1>Admin</h1><p>Authenticated admin view.</p><h2>Manual Poll</h2><p>POST /admin/poll with your session cookie to trigger a run.</p><h2>Recent Poll Runs</h2><table><thead><tr><th>ID</th><th>Status</th><th>Started</th><th>Completed</th></tr></thead><tbody>"#,
    );

    if runs.is_empty() {
        html.push_str(r#"<tr><td colspan="4">No runs yet.</td></tr>"#);
    } else {
        for run in runs {
            html.push_str(&format!(
                r#"<tr><td>{id}</td><td>{status}</td><td>{started}</td><td>{completed}</td></tr>"#,
                id = run.id,
                status = escape_html(&run.status),
                started = escape_html(&run.started_at_utc),
                completed = run
                    .completed_at_utc
                    .as_deref()
                    .map(escape_html)
                    .unwrap_or_else(|| "-".to_string())
            ));
        }
    }

    html.push_str(r#"</tbody></table><h2>Recent Poll Errors</h2><table><thead><tr><th>Run</th><th>Kind</th><th>Message</th><th>At</th></tr></thead><tbody>"#);
    if errors.is_empty() {
        html.push_str(r#"<tr><td colspan="4">No errors.</td></tr>"#);
    } else {
        for err in errors {
            html.push_str(&format!(
                r#"<tr><td>{run_id}</td><td>{kind}</td><td>{message}</td><td>{at}</td></tr>"#,
                run_id = err.run_id,
                kind = escape_html(&err.error_kind),
                message = escape_html(&err.message),
                at = escape_html(&err.at_utc),
            ));
        }
    }
    html.push_str("</tbody></table></main></body></html>");
    html
}

fn render_create_html(templates: &[CreateTemplateRow]) -> String {
    let mut html = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Create Meme</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><h1>Create Meme</h1><nav><a href="/">Gallery</a> <a href="/admin">Admin</a></nav></header><main class="create-layout"><section class="panel"><h2>Select Template</h2><div class="template-grid">"#,
    );

    if templates.is_empty() {
        html.push_str(
            r#"<p>No templates available yet. Trigger a poll from admin to load meme templates.</p>"#,
        );
    } else {
        for (index, template) in templates.iter().enumerate() {
            let checked = if index == 0 { " checked" } else { "" };
            let thumb = template
                .image_asset_id
                .map(|asset_id| {
                    format!(
                        r#"<img class="template-thumb" src="/media/image/{asset_id}" alt="{title} template"/>"#,
                        title = escape_html(&template.title),
                    )
                })
                .unwrap_or_else(|| {
                    r#"<div class="template-thumb placeholder">No Preview</div>"#.to_string()
                });

            html.push_str(&format!(
                r#"<label class="template-option"><input type="radio" name="base_meme_id" value="{meme_id}"{checked}/><span class="template-meta">#{rank}</span>{thumb}<span class="template-title">{title}</span></label>"#,
                meme_id = template.meme_id,
                checked = checked,
                rank = template.rank,
                thumb = thumb,
                title = escape_html(&template.title),
            ));
        }
    }

    html.push_str(
        r##"</div></section><section class="panel"><h2>Text Layers</h2><form id="create-form" class="create-form"><label>Top Text<input type="text" name="top_text" value="TOP TEXT" maxlength="120"/></label><label>Bottom Text<input type="text" name="bottom_text" value="BOTTOM TEXT" maxlength="120"/></label><label>Scale<input type="number" name="scale" value="4" min="1" max="10"/></label><label>Color<input type="color" name="color_hex" value="#ffffff"/></label><label>Bottom Y<input type="number" name="bottom_y" value="360" min="0" max="2000"/></label><label class="checkbox-row"><input type="checkbox" name="store" checked/> Store export in local DB</label><button type="submit">Render + Export</button></form><p id="create-result" class="status"></p></section></main><script>(function(){const form=document.getElementById('create-form');const result=document.getElementById('create-result');if(!form){return;}form.addEventListener('submit',async function(ev){ev.preventDefault();result.textContent='Rendering...';const selected=document.querySelector('input[name="base_meme_id"]:checked');const topText=form.top_text.value.trim();const bottomText=form.bottom_text.value.trim();const scale=Math.max(1,Number(form.scale.value)||4);const color=form.color_hex.value||'#ffffff';const bottomY=Math.max(0,Number(form.bottom_y.value)||360);const layers=[];if(topText){layers.push({text:topText,x:24,y:24,scale:scale,color_hex:color});}if(bottomText){layers.push({text:bottomText,x:24,y:bottomY,scale:scale,color_hex:color});}const payload={store:!!form.store.checked,base_meme_id:selected?Number(selected.value):null,layers:layers};try{const res=await fetch('/create/export',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(payload)});if(res.ok){result.textContent='Export accepted.';}else{result.textContent='Export failed ('+res.status+').';}}catch(_err){result.textContent='Network error while exporting.';}});})();</script></body></html>"##,
    );

    html
}

fn parse_hex_color(input: &str) -> Option<[u8; 4]> {
    let value = input.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&value[0..2], 16).ok()?;
    let g = u8::from_str_radix(&value[2..4], 16).ok()?;
    let b = u8::from_str_radix(&value[4..6], 16).ok()?;
    Some([r, g, b, 255])
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
