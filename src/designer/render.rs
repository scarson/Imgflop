use font8x8::UnicodeFonts;
use image::{ColorType, ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder, imageops::overlay};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLayer {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub scale: u32,
    pub color: [u8; 4],
}

impl Default for TextLayer {
    fn default() -> Self {
        Self {
            text: "IMGFLOP".to_string(),
            x: 24,
            y: 24,
            scale: 4,
            color: [255, 255, 255, 255],
        }
    }
}

pub fn render_png_bytes(layers: &[TextLayer]) -> Result<Vec<u8>, image::ImageError> {
    let mut image = RgbaImage::from_pixel(800, 450, Rgba([32, 35, 42, 255]));
    draw_background_gradient(&mut image);

    let base_layers: Vec<TextLayer> = if layers.is_empty() {
        vec![TextLayer::default()]
    } else {
        layers.to_vec()
    };

    for layer in &base_layers {
        draw_text_layer(&mut image, layer);
    }

    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        ColorType::Rgba8.into(),
    )?;
    Ok(bytes)
}

fn draw_background_gradient(image: &mut RgbaImage) {
    for y in 0..image.height() {
        let t = y as f32 / image.height() as f32;
        let r = (24.0 + 48.0 * t) as u8;
        let g = (28.0 + 35.0 * t) as u8;
        let b = (34.0 + 20.0 * t) as u8;
        for x in 0..image.width() {
            image.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
}

fn draw_text_layer(image: &mut RgbaImage, layer: &TextLayer) {
    let scale = layer.scale.max(1);
    let mut cursor_x = layer.x;
    let cursor_y = layer.y;
    for ch in layer.text.chars() {
        if ch == '\n' {
            cursor_x = layer.x;
            continue;
        }
        draw_char(image, ch, cursor_x, cursor_y, scale, layer.color);
        cursor_x = cursor_x.saturating_add(9 * scale);
    }
}

fn draw_char(image: &mut RgbaImage, ch: char, x: u32, y: u32, scale: u32, color: [u8; 4]) {
    let glyph = font8x8::BASIC_FONTS
        .get(ch)
        .or_else(|| font8x8::BASIC_FONTS.get(ch.to_ascii_uppercase()))
        .unwrap_or([0; 8]);

    let mut glyph_img = RgbaImage::from_pixel(8 * scale, 8 * scale, Rgba([0, 0, 0, 0]));
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if ((bits >> col) & 1) == 1 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        glyph_img.put_pixel(
                            (col as u32 * scale) + dx,
                            (row as u32 * scale) + dy,
                            Rgba(color),
                        );
                    }
                }
            }
        }
    }

    let shadow = RgbaImage::from_fn(glyph_img.width(), glyph_img.height(), |px, py| {
        let p = glyph_img.get_pixel(px, py);
        if p[3] == 0 {
            Rgba([0, 0, 0, 0])
        } else {
            Rgba([0, 0, 0, 180])
        }
    });

    overlay(image, &shadow, (x + scale) as i64, (y + scale) as i64);
    overlay(image, &glyph_img, x as i64, y as i64);
}
