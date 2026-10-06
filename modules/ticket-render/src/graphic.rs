use std::collections::HashMap;

use image::Rgba;

use crate::layout::Layout;
use crate::page::{err, render_rgba, PageError};
use crate::PreparedFace;

pub fn render_graphic(
    layout: &Layout,
    values: &HashMap<String, String>,
    dpi: i32,
    bg: &[u8],
) -> Result<Vec<u8>, PageError> {
    let image = render_rgba(layout, values, dpi, bg, None, None)?;
    Ok(rgba_graphic(&image))
}

pub fn render_graphic_prepared(
    layout: &Layout,
    values: &HashMap<String, String>,
    dpi: i32,
    background: &image::RgbaImage,
    faces: &[PreparedFace],
    scaled: Option<&image::RgbaImage>,
) -> Result<Vec<u8>, PageError> {
    let image = crate::render_rgba_prepared(layout, values, dpi, background, None, faces, scaled)?;
    Ok(rgba_graphic(&image))
}

pub fn png_graphic(png: &[u8]) -> Result<Vec<u8>, PageError> {
    let image = image::load_from_memory(png)
        .map_err(|error| err(error.to_string()))?
        .into_rgba8();
    Ok(rgba_graphic(&image))
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

pub fn rgba_graphic(image: &image::RgbaImage) -> Vec<u8> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let row_chars = width.div_ceil(4);
    let total = (width * height).div_ceil(8);
    let row_bytes = width.div_ceil(8);
    let mut out = Vec::with_capacity(24 + height * (row_chars + 1));
    out.extend_from_slice(format!("~DGBADGE,{total},{row_bytes},\n").as_bytes());
    let pixels = image.as_raw();
    for y in 0..height {
        let row = &pixels[y * width * 4..(y + 1) * width * 4];
        let mut nibble = 0u8;
        let mut count = 0u8;
        for px in row.chunks_exact(4) {
            nibble = (nibble << 1) | u8::from(is_black_bytes(px[0], px[1], px[2], px[3]));
            count += 1;
            if count == 4 {
                out.push(HEX[nibble as usize]);
                nibble = 0;
                count = 0;
            }
        }
        if count > 0 {
            out.push(HEX[(nibble << (4 - count)) as usize]);
        }
        out.push(b'\n');
    }
    out
}

fn is_black_bytes(red: u8, green: u8, blue: u8, alpha: u8) -> bool {
    if alpha == 255 {
        if red >= 200 && green >= 200 && blue >= 200 {
            return false;
        }
        if red <= 80 && green <= 80 && blue <= 80 {
            return true;
        }
    }
    is_black(Rgba([red, green, blue, alpha]))
}

fn is_black(pixel: Rgba<u8>) -> bool {
    let red = pixel[0] as f32 / 255.0;
    let green = pixel[1] as f32 / 255.0;
    let blue = pixel[2] as f32 / 255.0;
    let alpha = pixel[3] as f32 / 255.0;
    let luma = 0.2125 * blue + 0.7154 * green + 0.0721 * red;
    let value = 1.0 - ((1.0 - alpha) + luma * alpha);
    (value + 0.5).floor() >= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Alignment, Area, AreaKind, Layout, Rect, TextPart};
    use crate::render_png;
    use image::{DynamicImage, ImageFormat, RgbaImage};
    use std::io::Cursor;

    fn solid(color: [u8; 3]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(1, 1, image::Rgba([color[0], color[1], color[2], 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn fast_pixels_match_the_luma() {
        for red in (0..=255).step_by(17) {
            for green in (0..=255).step_by(17) {
                for blue in (0..=255).step_by(17) {
                    for alpha in [0, 80, 128, 200, 255] {
                        assert_eq!(
                            is_black_bytes(red, green, blue, alpha),
                            is_black(Rgba([red, green, blue, alpha])),
                            "{red},{green},{blue},{alpha}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn graphic_is_the_payload() {
        let layout = Layout {
            width: 8,
            height: 1,
            dots_per_point: 1.0,
            areas: Vec::new(),
        };
        let bytes = render_graphic(&layout, &HashMap::new(), 72, &solid([0, 0, 0])).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(!text.contains("^XA"));
        assert!(!text.contains("^FO"));
        assert!(!text.contains("^XZ"));
        assert!(!text.contains("^PR"));
        assert!(!text.contains("^MD"));
        assert!(!text.contains("~TA"));
        let mut lines = text.lines();
        let header = lines.next().unwrap();
        assert!(header.starts_with("~DG"), "{header}");
        assert!(header.contains(",1,1,"), "{header}");
        assert_eq!(lines.next().unwrap(), "FF");
        assert!(lines.next().is_none());
    }

    #[test]
    fn graphic_matches_the_png() {
        let layout = Layout {
            width: 144,
            height: 72,
            dots_per_point: 1.0,
            areas: vec![Area {
                rect: Rect {
                    start_x: 0.05,
                    start_y: 0.9,
                    end_x: 0.95,
                    end_y: 0.1,
                },
                alignment: Alignment { x: 0, y: 0 },
                rotation: 0,
                color: 0,
                text: "{{name}}".to_string(),
                condition: String::new(),
                kind: AreaKind::Text {
                    font_size: 36.0,
                    families: vec!["missing-font".to_string()],
                    weight: "normal".to_string(),
                    style: "normal".to_string(),
                    parts: vec![TextPart {
                        text: "{{name}}".to_string(),
                        is_break: false,
                        color: 0,
                        font_size: 36.0,
                        families: vec!["missing-font".to_string()],
                        weight: "normal".to_string(),
                        style: "normal".to_string(),
                    }],
                },
            }],
        };
        let values = HashMap::from([("name".to_string(), "Ann".to_string())]);
        let bg = solid([255, 255, 255]);
        let png = render_png(&layout, &values, 100, &bg, None).unwrap();
        let graphic = render_graphic(&layout, &values, 100, &bg).unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        let rows = rows_of(std::str::from_utf8(&graphic).unwrap(), image.width());
        assert_eq!(rows.len(), image.height() as usize);
        let mut saw_black = false;
        let mut saw_white = false;
        for (y, row) in rows.iter().enumerate() {
            assert_eq!(row.len(), image.width() as usize);
            for (x, bit) in row.iter().enumerate() {
                let pixel = *image.get_pixel(x as u32, y as u32);
                assert_eq!(*bit, is_black(pixel));
                saw_black |= *bit;
                saw_white |= !*bit;
            }
        }
        assert!(saw_black);
        assert!(saw_white);
    }

    fn rows_of(payload: &str, width: u32) -> Vec<Vec<bool>> {
        let mut lines = payload.lines();
        assert!(lines.next().unwrap().starts_with("~DG"));
        lines
            .map(|line| {
                let mut bits = Vec::new();
                for ch in line.chars() {
                    let nibble = ch.to_digit(16).unwrap();
                    for shift in (0..4).rev() {
                        bits.push((nibble >> shift) & 1 == 1);
                    }
                }
                bits.truncate(width as usize);
                bits
            })
            .collect()
    }
}
