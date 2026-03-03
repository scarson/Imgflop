CREATE TABLE IF NOT EXISTS poll_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL,
    started_at_utc TEXT NOT NULL,
    completed_at_utc TEXT,
    run_key TEXT
);

CREATE TABLE IF NOT EXISTS poll_run_errors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL,
    at_utc TEXT NOT NULL,
    severity TEXT NOT NULL,
    error_kind TEXT NOT NULL,
    message TEXT NOT NULL,
    context_json TEXT,
    FOREIGN KEY (run_id) REFERENCES poll_runs(id)
);

CREATE TABLE IF NOT EXISTS memes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    page_url TEXT,
    first_seen_at_utc TEXT NOT NULL,
    last_seen_at_utc TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    source_meme_id TEXT NOT NULL,
    meme_id INTEGER NOT NULL,
    raw_payload TEXT,
    FOREIGN KEY (meme_id) REFERENCES memes(id),
    UNIQUE (source, source_meme_id)
);

CREATE TABLE IF NOT EXISTS image_assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sha256 TEXT NOT NULL UNIQUE,
    disk_path TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    mime TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS top_state_current (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope TEXT NOT NULL,
    meme_id INTEGER NOT NULL,
    rank INTEGER NOT NULL,
    last_seen_run_id INTEGER NOT NULL,
    FOREIGN KEY (meme_id) REFERENCES memes(id),
    FOREIGN KEY (last_seen_run_id) REFERENCES poll_runs(id),
    UNIQUE (scope, meme_id)
);

CREATE TABLE IF NOT EXISTS top_state_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL,
    meme_id INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    old_rank INTEGER,
    new_rank INTEGER,
    at_utc TEXT NOT NULL,
    FOREIGN KEY (run_id) REFERENCES poll_runs(id),
    FOREIGN KEY (meme_id) REFERENCES memes(id)
);

CREATE TABLE IF NOT EXISTS created_memes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    base_meme_id INTEGER,
    output_asset_id INTEGER NOT NULL,
    stored INTEGER NOT NULL,
    created_at_utc TEXT NOT NULL,
    FOREIGN KEY (base_meme_id) REFERENCES memes(id),
    FOREIGN KEY (output_asset_id) REFERENCES image_assets(id)
);

CREATE TABLE IF NOT EXISTS created_meme_layers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_meme_id INTEGER NOT NULL,
    layer_index INTEGER NOT NULL,
    layer_text TEXT NOT NULL,
    x REAL NOT NULL,
    y REAL NOT NULL,
    style_json TEXT,
    FOREIGN KEY (created_meme_id) REFERENCES created_memes(id)
);

CREATE TABLE IF NOT EXISTS auth_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    username TEXT,
    at_utc TEXT NOT NULL,
    metadata_json TEXT
);
