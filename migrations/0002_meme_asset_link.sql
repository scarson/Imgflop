ALTER TABLE memes ADD COLUMN image_asset_id INTEGER;

CREATE INDEX IF NOT EXISTS idx_memes_image_asset_id ON memes(image_asset_id);
