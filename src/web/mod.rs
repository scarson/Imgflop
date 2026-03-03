use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::watch;

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
    shutdown_signal: Option<watch::Sender<bool>>,
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
        shutdown_signal: None,
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
        shutdown_signal: None,
    })
}

pub fn app_router_runtime(
    scheduler: Arc<Scheduler>,
    poll_runtime: Arc<PollRuntime>,
    auth: Arc<AuthService>,
    pool: SqlitePool,
    designer: DesignerService,
) -> Router {
    app_router_runtime_with_shutdown(scheduler, poll_runtime, auth, pool, designer, None)
}

pub fn app_router_runtime_with_shutdown(
    scheduler: Arc<Scheduler>,
    poll_runtime: Arc<PollRuntime>,
    auth: Arc<AuthService>,
    pool: SqlitePool,
    designer: DesignerService,
    shutdown_signal: Option<watch::Sender<bool>>,
) -> Router {
    app_router_with_state(AppState {
        scheduler,
        poll_runtime: Some(poll_runtime),
        auth,
        pool: Some(pool),
        designer: Some(designer),
        shutdown_signal,
    })
}

fn app_router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/", get(gallery_page))
        .route("/memes/{id}", get(meme_detail_page))
        .route("/create", get(create_page))
        .route("/create/{id}", get(create_designer_page))
        .route("/create/export", post(create_export))
        .route("/media/image/{id}", get(media_image))
        .route("/health", get(|| async { "ok" }))
        .route("/static/app.css", get(stylesheet))
        .route("/static/logo.png", get(logo_image))
        .route("/admin", get(admin_home))
        .route("/admin/login", get(admin_login_page).post(admin_login))
        .route("/admin/logout", post(admin_logout))
        .route("/admin/poll", post(trigger_manual_poll))
        .route("/admin/templates/upload", post(admin_upload_template))
        .route("/admin/shutdown", post(admin_shutdown))
        .with_state(state)
}

async fn gallery_page(
    State(state): State<AppState>,
    Query(query): Query<GalleryQuery>,
) -> Html<String> {
    let search = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(pool) = state.pool.as_ref() {
        match load_gallery_rows(pool, search).await {
            Ok(rows) => match load_local_meme_rows(pool, search).await {
                Ok(local_memes) => return Html(render_gallery_html(&rows, &local_memes, search)),
                Err(err) => tracing::error!(error = %err, "failed to load local meme data"),
            },
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
            Ok(rows) => return Html(render_create_template_picker_html(&rows)),
            Err(err) => tracing::error!(error = %err, "failed to load create templates"),
        }
    }

    Html(routes::create::render().to_string())
}

async fn create_designer_page(
    State(state): State<AppState>,
    Path(template_id): Path<i64>,
) -> impl IntoResponse {
    let Some(pool) = state.pool.as_ref() else {
        return Html(routes::create::render().to_string()).into_response();
    };

    match load_create_template_detail(pool, template_id).await {
        Ok(Some(template)) => Html(render_create_designer_html(&template)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!(error = %err, template_id, "failed to load template detail");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_export(State(state): State<AppState>, body: Bytes) -> impl IntoResponse {
    let parsed = if body.is_empty() {
        CreateExportRequest::default()
    } else {
        serde_json::from_slice::<CreateExportRequest>(&body).unwrap_or_default()
    };

    let Some(designer) = state.designer.as_ref() else {
        return (
            StatusCode::OK,
            Json(CreateExportResponse {
                stored: false,
                created_id: None,
            }),
        )
            .into_response();
    };

    let layers: Vec<TextLayer> = parsed
        .layers
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect();
    let should_store = parsed.store.unwrap_or(true);
    let should_download = parsed.download.unwrap_or(false);

    if should_download {
        let png = match designer
            .render_png_from_template(parsed.base_meme_id, &layers)
            .await
        {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::error!(error = %err, "create export render failed");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        let created_id = if should_store {
            match designer
                .export_from_template(parsed.base_meme_id, true, &layers)
                .await
            {
                Ok(id) => id,
                Err(err) => {
                    tracing::error!(error = %err, "create export store failed");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        } else {
            None
        };

        let mut response = (StatusCode::OK, png).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"imgflop-export.png\""),
        );
        if let Some(id) = created_id
            && let Ok(value) = HeaderValue::from_str(&id.to_string())
        {
            response.headers_mut().insert("x-imgflop-created-id", value);
        }
        return response;
    }

    match designer
        .export_from_template(parsed.base_meme_id, should_store, &layers)
        .await
    {
        Ok(created_id) => (
            StatusCode::OK,
            Json(CreateExportResponse {
                stored: created_id.is_some(),
                created_id,
            }),
        )
            .into_response(),
        Err(err) => {
            tracing::error!(error = %err, "create export failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
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

async fn logo_image() -> ([(&'static str, &'static str); 1], &'static [u8]) {
    (
        [("content-type", "image/png")],
        &include_bytes!("../../img/imgflop-synthwave-logo-1.png")[..],
    )
}

async fn admin_login_page() -> Html<String> {
    Html(render_admin_login_html(None))
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
        Redirect::to("/admin/login").into_response()
    }
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Default, Deserialize)]
struct GalleryQuery {
    q: Option<String>,
}

async fn admin_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let expects_json = request_is_json(&headers);
    let parsed = parse_login_request(&headers, &body);
    let payload = match parsed {
        Ok(value) => value,
        Err(_) => {
            return if expects_json {
                StatusCode::BAD_REQUEST.into_response()
            } else {
                Html(render_admin_login_html(Some("Invalid login payload"))).into_response()
            };
        }
    };

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
            if !expects_json {
                *response.status_mut() = StatusCode::SEE_OTHER;
                response
                    .headers_mut()
                    .insert(header::LOCATION, HeaderValue::from_static("/admin"));
            }
            response
        }
        Err(_) => {
            if expects_json {
                StatusCode::UNAUTHORIZED.into_response()
            } else {
                Html(render_admin_login_html(Some(
                    "Invalid username or password",
                )))
                .into_response()
            }
        }
    }
}

async fn admin_logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = extract_session_token(&headers) {
        state.auth.logout_token(&token);
    }

    let mut response = Redirect::to("/").into_response();
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

async fn admin_upload_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !state.auth.is_authenticated_headers(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(designer) = state.designer.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    let mut title: Option<String> = None;
    let mut mime: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(value) => value,
            Err(err) => {
                tracing::error!(error = %err, "template upload field read failed");
                return StatusCode::BAD_REQUEST.into_response();
            }
        };
        let Some(field) = field else {
            break;
        };

        match field.name() {
            Some("title") => {
                if let Ok(value) = field.text().await {
                    title = Some(value);
                }
            }
            Some("file") => {
                mime = field.content_type().map(str::to_string);
                if let Ok(value) = field.bytes().await {
                    let bytes = value.to_vec();
                    if bytes.len() > 10 * 1024 * 1024 {
                        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
                    }
                    file_bytes = Some(bytes);
                }
            }
            _ => {}
        }
    }

    let Some(file_bytes) = file_bytes else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let title = title.unwrap_or_else(|| "Uploaded Template".to_string());
    let mime = mime.unwrap_or_else(|| "application/octet-stream".to_string());

    match designer.upload_template(&title, &mime, &file_bytes).await {
        Ok(meme_id) => Redirect::to(&format!("/create/{meme_id}")).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "template upload persistence failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn admin_shutdown(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !state.auth.is_authenticated_headers(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(signal) = state.shutdown_signal.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    let _ = signal.send(true);
    Html(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Shutting Down</title><link rel="stylesheet" href="/static/app.css"/></head><body><main><section class="panel"><h1>Server Shutdown Requested</h1><p>Imgflop is shutting down gracefully. You can close this tab.</p></section></main></body></html>"#.to_string(),
    )
    .into_response()
}

#[derive(Debug, Default, Deserialize)]
struct CreateExportRequest {
    store: Option<bool>,
    download: Option<bool>,
    base_meme_id: Option<i64>,
    layers: Option<Vec<CreateLayer>>,
}

#[derive(Debug, Serialize)]
struct CreateExportResponse {
    stored: bool,
    created_id: Option<i64>,
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
    rank: Option<i64>,
    is_local_template: bool,
    title: String,
    image_asset_id: Option<i64>,
}

#[derive(Debug)]
struct CreateTemplateDetail {
    meme_id: i64,
    title: String,
    image_asset_id: i64,
}

#[derive(Debug)]
struct LocalMemeRow {
    id: i64,
    created_at_utc: String,
    output_asset_id: i64,
    base_title: Option<String>,
    first_layer_text: Option<String>,
}

async fn load_gallery_rows(
    pool: &SqlitePool,
    search: Option<&str>,
) -> Result<Vec<GalleryRow>, sqlx::Error> {
    let search_like = search.map(|value| format!("%{}%", value.to_ascii_lowercase()));
    let rows = sqlx::query_as::<_, (i64, i64, String, Option<String>, Option<i64>)>(
        r#"
        SELECT t.meme_id, t.rank, m.title, m.page_url, m.image_asset_id
        FROM top_state_current t
        JOIN memes m ON m.id = t.meme_id
        WHERE t.scope = 'api'
          AND (? IS NULL OR LOWER(m.title) LIKE ?)
        ORDER BY t.rank ASC
        LIMIT 200
        "#,
    )
    .bind(search_like.as_deref())
    .bind(search_like.as_deref())
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

async fn load_local_meme_rows(
    pool: &SqlitePool,
    search: Option<&str>,
) -> Result<Vec<LocalMemeRow>, sqlx::Error> {
    let search_like = search.map(|value| format!("%{}%", value.to_ascii_lowercase()));
    let rows = sqlx::query_as::<_, (i64, String, i64, Option<String>, Option<String>)>(
        r#"
        SELECT c.id,
               c.created_at_utc,
               c.output_asset_id,
               base.title,
               l.layer_text
        FROM created_memes c
        LEFT JOIN memes base ON base.id = c.base_meme_id
        LEFT JOIN created_meme_layers l
          ON l.created_meme_id = c.id
         AND l.layer_index = 0
        WHERE c.stored = 1
          AND (
              ? IS NULL
              OR LOWER(COALESCE(base.title, '')) LIKE ?
              OR LOWER(COALESCE(l.layer_text, '')) LIKE ?
          )
        ORDER BY c.id DESC
        LIMIT 60
        "#,
    )
    .bind(search_like.as_deref())
    .bind(search_like.as_deref())
    .bind(search_like.as_deref())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, created_at_utc, output_asset_id, base_title, first_layer_text)| LocalMemeRow {
                id,
                created_at_utc,
                output_asset_id,
                base_title,
                first_layer_text,
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
    let rows = sqlx::query_as::<_, (i64, Option<i64>, i64, String, Option<i64>)>(
        r#"
        SELECT m.id,
               t.rank,
               CASE WHEN t.id IS NULL THEN 1 ELSE 0 END AS is_local_template,
               m.title,
               m.image_asset_id
        FROM memes m
        LEFT JOIN top_state_current t
          ON t.meme_id = m.id
         AND t.scope = 'api'
        WHERE m.image_asset_id IS NOT NULL
          AND (
                t.id IS NOT NULL
                OR EXISTS (
                    SELECT 1 FROM source_records s
                    WHERE s.meme_id = m.id
                      AND s.source = 'admin_upload'
                )
          )
        ORDER BY is_local_template ASC, t.rank ASC, m.id DESC
        LIMIT 200
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(meme_id, rank, is_local_template, title, image_asset_id)| CreateTemplateRow {
                meme_id,
                rank,
                is_local_template: is_local_template == 1,
                title,
                image_asset_id,
            },
        )
        .collect())
}

async fn load_create_template_detail(
    pool: &SqlitePool,
    template_id: i64,
) -> Result<Option<CreateTemplateDetail>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, String, i64)>(
        r#"
        SELECT m.id, m.title, m.image_asset_id
        FROM memes m
        WHERE m.id = ?
          AND m.image_asset_id IS NOT NULL
        "#,
    )
    .bind(template_id)
    .fetch_optional(pool)
    .await?;

    Ok(
        row.map(|(meme_id, title, image_asset_id)| CreateTemplateDetail {
            meme_id,
            title,
            image_asset_id,
        }),
    )
}

fn render_gallery_html(
    rows: &[GalleryRow],
    local_memes: &[LocalMemeRow],
    search: Option<&str>,
) -> String {
    let search_value = search.unwrap_or_default();
    let mut html = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Imgflop - Top Memes</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><h1><a href="/" class="brand-link">Imgflop - Top Memes</a></h1><nav><a href="/create">Create Meme</a> <a href="/admin">Admin</a></nav></header><main><section class="panel"><form class="searchbar" method="get" action="/"><label for="q">Search Top Memes</label><div class="search-controls"><input id="q" type="search" name="q" placeholder="Search by title" value="" /><button type="submit">Search</button><button type="button" id="clear-search">Clear</button><button type="button" id="back-search">Back</button></div></form></section><section><h2>Imgflip Feed</h2><div class="gallery">"#,
    );
    html = html.replacen(
        "value=\"\"",
        &format!(r#"value="{}""#, escape_html(search_value)),
        1,
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
                        r#"<button type="button" class="thumb-button" data-modal-src="/media/image/{asset_id}" data-modal-title="{title}"><img class="thumb" src="/media/image/{asset_id}" alt="{title} preview" loading="lazy"/></button>"#,
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

    html.push_str("</div></section><section><h2>Local Memes</h2><div class=\"gallery\">");
    if local_memes.is_empty() {
        html.push_str(r#"<article class="card">No local memes yet. Use Create Meme and store an export.</article>"#);
    } else {
        for meme in local_memes {
            let display_name = local_meme_display_name(meme);
            let title = escape_html(&display_name);
            let created_at = render_epoch_time(&meme.created_at_utc, false);
            html.push_str(&format!(
                r#"<article class="card"><button type="button" class="thumb-button" data-modal-src="/media/image/{asset_id}" data-modal-title="{title}"><img class="thumb" src="/media/image/{asset_id}" alt="{title}" loading="lazy"/></button><h3>{title}</h3><p>Created at {created_at}</p></article>"#,
                asset_id = meme.output_asset_id,
                title = title,
                created_at = created_at,
            ));
        }
    }
    html.push_str(
        r##"</div></section><div id="image-modal" class="modal hidden"><div class="modal-backdrop" data-close-modal="true"></div><div class="modal-content"><button type="button" class="modal-close" data-close-modal="true">Close</button><h3 id="modal-title"></h3><img id="modal-image" class="modal-image" alt="Meme preview"/></div></div></main><script>(function(){const form=document.querySelector('form.searchbar');const input=document.getElementById('q');const clearBtn=document.getElementById('clear-search');const backBtn=document.getElementById('back-search');if(clearBtn&&input){clearBtn.addEventListener('click',function(){input.value='';if(form){form.submit();return;}input.focus();});}if(backBtn){backBtn.addEventListener('click',function(){if(window.history.length>1){window.history.back();}else{window.location.href='/';}});}function pad(v){return String(v).padStart(2,'0');}function fmt(epoch,withSeconds){const d=new Date(Number(epoch)*1000);if(Number.isNaN(d.getTime())){return epoch;}const y=d.getFullYear();const m=pad(d.getMonth()+1);const day=pad(d.getDate());const hh=pad(d.getHours());const mm=pad(d.getMinutes());if(withSeconds){return y+'-'+m+'-'+day+' '+hh+':'+mm+':'+pad(d.getSeconds());}return y+'-'+m+'-'+day+' '+hh+':'+mm;}document.querySelectorAll('time[data-epoch]').forEach(function(el){const epoch=el.getAttribute('data-epoch')||el.textContent||'';const withSeconds=el.classList.contains('ts-second');el.textContent=fmt(epoch,withSeconds);});const modal=document.getElementById('image-modal');const modalImg=document.getElementById('modal-image');const modalTitle=document.getElementById('modal-title');function closeModal(){if(modal){modal.classList.add('hidden');}if(modalImg){modalImg.removeAttribute('src');}}document.querySelectorAll('.thumb-button').forEach(function(btn){btn.addEventListener('click',function(){const src=btn.getAttribute('data-modal-src');const title=btn.getAttribute('data-modal-title')||'Preview';if(!src||!modal||!modalImg||!modalTitle){return;}modalImg.setAttribute('src',src);modalTitle.textContent=title;modal.classList.remove('hidden');});});document.querySelectorAll('[data-close-modal="true"]').forEach(function(el){el.addEventListener('click',closeModal);});document.addEventListener('keydown',function(ev){if(ev.key==='Escape'){closeModal();}});})();</script></body></html>"##,
    );
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
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Imgflop - Admin</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><div class="left-actions"><form method="post" action="/admin/logout"><button type="submit">Logout</button></form></div><h1><a href="/" class="brand-link">Imgflop - Admin</a></h1><nav><a href="/">Gallery</a> <a href="/create">Create Meme</a></nav></header><main><section class="panel"><h2>Manual Poll</h2><form method="post" action="/admin/poll"><button type="submit">Run Poll Now</button></form></section><section class="panel"><h2>Upload Template</h2><form method="post" action="/admin/templates/upload" enctype="multipart/form-data" class="create-form narrow-form"><label>Template Name<input type="text" name="title" maxlength="120" required/></label><label>Image File<input type="file" name="file" accept="image/*" required/></label><button type="submit">Upload Template</button></form></section><section class="panel"><h2>Recent Poll Runs</h2><table><thead><tr><th>ID</th><th>Status</th><th>Started</th><th>Completed</th></tr></thead><tbody>"#,
    );

    if runs.is_empty() {
        html.push_str(r#"<tr><td colspan="4">No runs yet.</td></tr>"#);
    } else {
        for run in runs {
            let started = render_epoch_time(&run.started_at_utc, true);
            let completed = run
                .completed_at_utc
                .as_deref()
                .map(|value| render_epoch_time(value, true))
                .unwrap_or_else(|| "-".to_string());
            html.push_str(&format!(
                r#"<tr><td>{id}</td><td>{status}</td><td>{started}</td><td>{completed}</td></tr>"#,
                id = run.id,
                status = escape_html(&run.status),
                started = started,
                completed = completed
            ));
        }
    }

    html.push_str(r#"</tbody></table></section><section class="panel"><h2>Recent Poll Errors</h2><table><thead><tr><th>Run</th><th>Kind</th><th>Message</th><th>At</th></tr></thead><tbody>"#);
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
    html.push_str(r#"</tbody></table></section><section class="panel admin-actions"><form method="post" action="/admin/shutdown" onsubmit="return confirm('Shut down Imgflop server now?');"><button type="submit" class="danger">Shut Down Server</button></form></section></main><script>(function(){function pad(v){return String(v).padStart(2,'0');}function fmt(epoch){const d=new Date(Number(epoch)*1000);if(Number.isNaN(d.getTime())){return epoch;}return d.getFullYear()+'-'+pad(d.getMonth()+1)+'-'+pad(d.getDate())+' '+pad(d.getHours())+':'+pad(d.getMinutes())+':'+pad(d.getSeconds());}document.querySelectorAll('time.ts-second[data-epoch]').forEach(function(el){const raw=el.getAttribute('data-epoch')||el.textContent||'';el.textContent=fmt(raw);});})();</script></body></html>"#);
    html
}

fn render_create_template_picker_html(templates: &[CreateTemplateRow]) -> String {
    let mut html = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Create Meme</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><h1>Create Meme</h1><nav><a href="/">Gallery</a> <a href="/admin">Admin</a></nav></header><main><section class="panel"><h2>Select Template</h2><p>Pick a template to open the full designer.</p><div class="template-grid">"#,
    );

    if templates.is_empty() {
        html.push_str(
            r#"<p>No templates available yet. Trigger a poll from admin to load meme templates.</p>"#,
        );
    } else {
        for template in templates {
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

            let rank_label = template
                .rank
                .map(|rank| format!("#{rank}"))
                .unwrap_or_else(|| "Local".to_string());
            let origin = if template.is_local_template {
                "Uploaded template"
            } else {
                "Imgflip feed"
            };
            html.push_str(&format!(
                r#"<a class="template-option" href="/create/{meme_id}"><span class="template-meta">{rank_label} · {origin}</span>{thumb}<span class="template-title">{title}</span></a>"#,
                meme_id = template.meme_id,
                rank_label = rank_label,
                origin = origin,
                thumb = thumb,
                title = escape_html(&template.title),
            ));
        }
    }

    html.push_str("</div></section></main></body></html>");

    html
}

fn render_create_designer_html(template: &CreateTemplateDetail) -> String {
    format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Designer</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><h1>Designer: {title}</h1><nav><a href="/create">Template Picker</a> <a href="/">Gallery</a> <a href="/admin">Admin</a></nav></header><main class="designer-layout"><section class="panel"><h2>Live Preview</h2><div id="preview-stage" class="preview-stage"><img id="preview-base" src="/media/image/{asset_id}" alt="{title} template"/><div id="overlay-top" class="preview-text">TOP TEXT</div><div id="overlay-bottom" class="preview-text">BOTTOM TEXT</div></div></section><section class="panel"><h2>Layer Editor</h2><form id="designer-form" class="create-form"><input type="hidden" name="base_meme_id" value="{meme_id}"/><label>Top Text<input type="text" name="top_text" value="TOP TEXT" maxlength="120"/></label><label>Bottom Text<input type="text" name="bottom_text" value="BOTTOM TEXT" maxlength="120"/></label><label>Text Scale<input type="range" name="scale" value="4" min="1" max="12"/></label><label>Text Color<input type="color" name="color_hex" value="#ffffff"/></label><label>Top X<input type="number" name="top_x" value="24" min="0" max="3000"/></label><label>Top Y<input type="number" name="top_y" value="24" min="0" max="3000"/></label><label>Bottom X<input type="number" name="bottom_x" value="24" min="0" max="3000"/></label><label>Bottom Y<input type="number" name="bottom_y" value="360" min="0" max="3000"/></label><label class="checkbox-row"><input type="checkbox" name="store" checked/> Store in Imgflop</label><button type="submit">Render + Export</button></form><p id="designer-status" class="status"></p></section></main><script>(function(){{const form=document.getElementById('designer-form');const top=document.getElementById('overlay-top');const bottom=document.getElementById('overlay-bottom');const status=document.getElementById('designer-status');function sync(){{const scale=Math.max(1,Number(form.scale.value)||4);const color=form.color_hex.value||'#ffffff';top.textContent=form.top_text.value||'';bottom.textContent=form.bottom_text.value||'';top.style.left=(Number(form.top_x.value)||0)+'px';top.style.top=(Number(form.top_y.value)||0)+'px';bottom.style.left=(Number(form.bottom_x.value)||0)+'px';bottom.style.top=(Number(form.bottom_y.value)||0)+'px';top.style.color=color;bottom.style.color=color;top.style.fontSize=(scale*10)+'px';bottom.style.fontSize=(scale*10)+'px';}}form.addEventListener('input',sync);sync();form.addEventListener('submit',async function(ev){{ev.preventDefault();status.textContent='Rendering...';const scale=Math.max(1,Number(form.scale.value)||4);const color=form.color_hex.value||'#ffffff';const layers=[];if((form.top_text.value||'').trim()){{layers.push({{text:form.top_text.value.trim(),x:Number(form.top_x.value)||24,y:Number(form.top_y.value)||24,scale:scale,color_hex:color}});}}if((form.bottom_text.value||'').trim()){{layers.push({{text:form.bottom_text.value.trim(),x:Number(form.bottom_x.value)||24,y:Number(form.bottom_y.value)||360,scale:scale,color_hex:color}});}}const payload={{store:!!form.store.checked,download:true,base_meme_id:Number(form.base_meme_id.value),layers:layers}};try{{const res=await fetch('/create/export',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify(payload)}});if(!res.ok){{status.textContent='Export failed ('+res.status+').';return;}}const blob=await res.blob();if(!blob||blob.size===0){{status.textContent='Export failed (empty image).';return;}}const url=window.URL.createObjectURL(blob);const link=document.createElement('a');link.href=url;link.download='imgflop-export.png';document.body.appendChild(link);link.click();link.remove();window.setTimeout(function(){{window.URL.revokeObjectURL(url);}},1000);const createdId=res.headers.get('x-imgflop-created-id');if(form.store.checked&&createdId){{status.textContent='Exported PNG and stored meme #'+createdId+'.';}}else if(form.store.checked){{status.textContent='Exported PNG and stored in Imgflop.';}}else{{status.textContent='Exported PNG download.';}}}}catch(_err){{status.textContent='Network error while exporting.';}}}});}})();</script></body></html>"##,
        title = escape_html(&template.title),
        asset_id = template.image_asset_id,
        meme_id = template.meme_id
    )
}

fn render_admin_login_html(error: Option<&str>) -> String {
    let error_html = error
        .map(|value| format!(r#"<p class="status error">{}</p>"#, escape_html(value)))
        .unwrap_or_default();
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width, initial-scale=1"/><title>Admin Login</title><link rel="stylesheet" href="/static/app.css"/></head><body><header class="topbar"><h1>Admin Login</h1><nav><a href="/">Gallery</a> <a href="/create">Create Meme</a></nav></header><main><section class="panel auth-panel"><h2>Sign In</h2>{error_html}<form method="post" action="/admin/login" class="create-form narrow-form"><label>Username<input type="text" name="username" required/></label><label>Password<input type="password" name="password" required/></label><button type="submit">Login</button></form></section></main></body></html>"#,
    )
}

fn local_meme_display_name(meme: &LocalMemeRow) -> String {
    let base = meme
        .base_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let first = meme
        .first_layer_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (base, first) {
        (Some(base), Some(first)) => format!("{base}: {}", truncate_label(first, 48)),
        (Some(base), None) => base.to_string(),
        (None, Some(first)) => format!("Local Meme: {}", truncate_label(first, 48)),
        (None, None) => format!("Local Meme #{}", meme.id),
    }
}

fn truncate_label(input: &str, max_chars: usize) -> String {
    let count = input.chars().count();
    if count <= max_chars {
        return input.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars.saturating_sub(3) {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn render_epoch_time(epoch: &str, with_seconds: bool) -> String {
    if epoch.parse::<i64>().is_ok() {
        let class = if with_seconds {
            "ts-second"
        } else {
            "ts-minute"
        };
        let escaped = escape_html(epoch);
        return format!(r#"<time class="{class}" data-epoch="{escaped}">{escaped}</time>"#);
    }

    escape_html(epoch)
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

fn request_is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("application/json"))
        .unwrap_or(false)
}

fn parse_login_request(headers: &HeaderMap, body: &[u8]) -> Result<LoginRequest, String> {
    if request_is_json(headers) {
        return serde_json::from_slice(body).map_err(|err| err.to_string());
    }

    serde_urlencoded::from_bytes(body).map_err(|err| err.to_string())
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
