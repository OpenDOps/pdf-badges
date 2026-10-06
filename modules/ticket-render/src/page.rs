use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use fontdue::{Font, FontSettings};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use crate::codes::{symbol, Symbol};
use crate::layout::{Area, AreaKind, Layout, Rect, TextPart};
use crate::replace_areas;

const TEST_FONT: &[u8] = include_bytes!("../fonts/dejavu-sans/dejavu-sans-400-normal.ttf");

#[derive(Debug)]
pub struct PageError {
    message: String,
}

impl PageError {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PageError {}

pub(crate) fn err(message: impl Into<String>) -> PageError {
    PageError {
        message: message.into(),
    }
}

pub fn render_png(
    layout: &Layout,
    values: &HashMap<String, String>,
    dpi: i32,
    bg: &[u8],
    photo: Option<&[u8]>,
) -> Result<Vec<u8>, PageError> {
    let (width, height) = page_px(layout, dpi)?;
    let source = image::load_from_memory(bg).map_err(|error| err(error.to_string()))?;
    let mut image = image::imageops::resize(
        &source.to_rgba8(),
        width,
        height,
        image::imageops::FilterType::Nearest,
    );
    for area in replace_areas(&layout.areas, values) {
        match &area.kind {
            AreaKind::Text { .. } => draw_text(&mut image, &area, dpi, layout.dots_per_point)?,
            AreaKind::Barcode { barcode_type } => {
                draw_barcode(&mut image, &area, barcode_type)?;
            }
            AreaKind::Photo => draw_photo(&mut image, &area, photo)?,
        }
    }
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| err(error.to_string()))?;
    Ok(bytes)
}

pub(crate) fn page_px(layout: &Layout, dpi: i32) -> Result<(u32, u32), PageError> {
    if dpi <= 0 || !layout.dots_per_point.is_finite() || layout.dots_per_point == 0.0 {
        return Err(err("page size"));
    }
    let width = side_px(layout.width, dpi, layout.dots_per_point)?;
    let height = side_px(layout.height, dpi, layout.dots_per_point)?;
    Ok((width, height))
}

fn side_px(length: i32, dpi: i32, dots_per_point: f32) -> Result<u32, PageError> {
    let px = (length.wrapping_mul(dpi) as f32) / dots_per_point / 72.0;
    let px = px.round();
    if !px.is_finite() || px < 1.0 || px > u32::MAX as f32 {
        return Err(err(format!("page side {px}")));
    }
    Ok(px as u32)
}

fn font_px(font_size: f32, dpi: i32, dots_per_point: f32) -> f32 {
    let px = (font_size * dpi as f32 / dots_per_point / 72.0).round();
    if px.is_finite() && px >= 1.0 {
        px
    } else {
        1.0
    }
}

pub(crate) fn area_px(rect: &Rect, width: u32, height: u32) -> (i32, i32, i32, i32) {
    let width = width as f64;
    let height_px = height as f64;
    let start_x = (width * rect.start_x + 0.5).floor() as i32;
    let end_x = (width * rect.end_x + 0.5).floor() as i32;
    let start_y = height as i32 - (height_px * rect.start_y + 0.5).floor() as i32;
    let end_y = height as i32 - (height_px * rect.end_y + 0.5).floor() as i32;
    (start_x, start_y, end_x, end_y)
}

struct Run<'a> {
    text: &'a str,
    font_size: f32,
    families: &'a [String],
    weight: &'a str,
    style: &'a str,
    color: i32,
}

fn draw_text(
    image: &mut RgbaImage,
    area: &Area,
    dpi: i32,
    dots_per_point: f32,
) -> Result<(), PageError> {
    let AreaKind::Text {
        font_size,
        families,
        weight,
        style,
        parts,
    } = &area.kind
    else {
        return Ok(());
    };
    let (start_x, start_y, end_x, end_y) = area_px(&area.rect, image.width(), image.height());
    let box_w = end_x - start_x;
    let box_h = end_y - start_y;
    if box_w <= 0 || box_h <= 0 {
        return Ok(());
    }
    let lines = text_lines(
        area.text.as_str(),
        *font_size,
        families,
        weight,
        style,
        area.color,
        parts,
    );
    if lines
        .iter()
        .all(|line| line.iter().all(|run| run.text.is_empty()))
    {
        return Ok(());
    }

    let rotated = area.rotation == 1 || area.rotation == 3;
    let content_w = if rotated { box_h } else { box_w } as f32;
    let content_h = if rotated { box_w } else { box_h } as f32;
    let center_x = (start_x + end_x) as f32 / 2.0;
    let center_y = (start_y + end_y) as f32 / 2.0;

    let mut scale = 1.0f32;
    let mut measured = measure_lines(&lines, dpi, dots_per_point, scale)?;
    let width = measured.iter().map(|line| line.width).fold(0.0, f32::max);
    let height = measured.iter().map(|line| line.line_h).sum::<f32>();
    if width > content_w || height > content_h {
        let fit_w = if width > 0.0 { content_w / width } else { 1.0 };
        let fit_h = if height > 0.0 {
            content_h / height
        } else {
            1.0
        };
        scale = fit_w.min(fit_h).min(1.0);
        if scale < 1.0 {
            measured = measure_lines(&lines, dpi, dots_per_point, scale)?;
        }
    }
    let block_h = measured.iter().map(|line| line.line_h).sum::<f32>();
    let mut top = match area.alignment.y {
        1 => (content_h - block_h) / 2.0,
        3 => content_h - block_h,
        _ => 0.0,
    };
    for line in measured {
        let left = match area.alignment.x {
            1 => (content_w - line.width) / 2.0,
            2 => content_w - line.width,
            _ => 0.0,
        };
        for glyph in line.glyphs {
            blit(
                image,
                &glyph,
                left,
                top,
                content_w,
                content_h,
                center_x,
                center_y,
                area.rotation,
            );
        }
        top += line.line_h;
    }
    Ok(())
}

fn draw_barcode(image: &mut RgbaImage, area: &Area, barcode_type: &str) -> Result<(), PageError> {
    let Some((start_x, start_y, end_x, end_y)) = positive_box(image, &area.rect) else {
        return Ok(());
    };
    match symbol(barcode_type, &area.text)? {
        Symbol::Bars(modules) => paint_bars(image, start_x, start_y, end_x, end_y, &modules),
        Symbol::Qr { width, modules } => {
            paint_matrix(image, start_x, start_y, end_x, end_y, width, &modules);
        }
    }
    Ok(())
}

fn draw_photo(image: &mut RgbaImage, area: &Area, photo: Option<&[u8]>) -> Result<(), PageError> {
    let Some(bytes) = photo else {
        return Ok(());
    };
    let Some((start_x, start_y, end_x, end_y)) = positive_box(image, &area.rect) else {
        return Ok(());
    };
    let source = image::load_from_memory(bytes).map_err(|error| err(error.to_string()))?;
    let width = (end_x - start_x) as u32;
    let height = (end_y - start_y) as u32;
    let scaled = image::imageops::resize(
        &source.to_rgba8(),
        width,
        height,
        image::imageops::FilterType::Nearest,
    );
    for y in 0..height {
        for x in 0..width {
            image.put_pixel(
                start_x as u32 + x,
                start_y as u32 + y,
                *scaled.get_pixel(x, y),
            );
        }
    }
    Ok(())
}

fn positive_box(image: &RgbaImage, rect: &Rect) -> Option<(i32, i32, i32, i32)> {
    let (start_x, start_y, end_x, end_y) = area_px(rect, image.width(), image.height());
    if end_x <= start_x || end_y <= start_y {
        None
    } else {
        Some((start_x, start_y, end_x, end_y))
    }
}

fn paint_bars(
    image: &mut RgbaImage,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    modules: &[bool],
) {
    let span = (end_x - start_x) as u32;
    let count = modules.len() as u32;
    if span == 0 || count == 0 {
        return;
    }
    for (index, on) in modules.iter().enumerate() {
        if !on {
            continue;
        }
        let left = start_x + (index as u32 * span / count) as i32;
        let right = start_x + ((index as u32 + 1) * span / count) as i32;
        for y in start_y..end_y {
            for x in left..right {
                fill(image, x, y, [0, 0, 0]);
            }
        }
    }
}

fn paint_matrix(
    image: &mut RgbaImage,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    width: usize,
    modules: &[bool],
) {
    if width == 0 {
        return;
    }
    let box_w = (end_x - start_x) as u32;
    let box_h = (end_y - start_y) as u32;
    let module = box_w.min(box_h) / width as u32;
    if module == 0 {
        return;
    }
    let span = module * width as u32;
    let origin_x = start_x + (box_w as i32 - span as i32) / 2;
    let origin_y = start_y + (box_h as i32 - span as i32) / 2;
    for y in 0..width {
        for x in 0..width {
            if !modules[y * width + x] {
                continue;
            }
            let left = origin_x + x as i32 * module as i32;
            let top = origin_y + y as i32 * module as i32;
            for py in top..top + module as i32 {
                for px in left..left + module as i32 {
                    fill(image, px, py, [0, 0, 0]);
                }
            }
        }
    }
}

fn fill(image: &mut RgbaImage, x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as u32;
    let y = y as u32;
    if x < image.width() && y < image.height() {
        image.put_pixel(x, y, Rgba([color[0], color[1], color[2], 255]));
    }
}

fn text_lines<'a>(
    text: &'a str,
    font_size: f32,
    families: &'a [String],
    weight: &'a str,
    style: &'a str,
    color: i32,
    parts: &'a [TextPart],
) -> Vec<Vec<Run<'a>>> {
    if parts.is_empty() {
        return vec![vec![Run {
            text,
            font_size,
            families,
            weight,
            style,
            color,
        }]];
    }
    let mut lines = Vec::new();
    let mut line = Vec::new();
    for part in parts {
        if part.is_break && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if part.is_break {
            continue;
        }
        line.push(Run {
            text: part.text.as_str(),
            font_size: part.font_size,
            families: part.families.as_slice(),
            weight: part.weight.as_str(),
            style: part.style.as_str(),
            color: part.color,
        });
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

struct Glyph {
    x: f32,
    y: f32,
    width: usize,
    height: usize,
    bitmap: Vec<u8>,
    color: [u8; 3],
}

struct MeasuredLine {
    width: f32,
    line_h: f32,
    glyphs: Vec<Glyph>,
}

fn measure_lines(
    lines: &[Vec<Run<'_>>],
    dpi: i32,
    dots_per_point: f32,
    scale: f32,
) -> Result<Vec<MeasuredLine>, PageError> {
    let mut measured = Vec::new();
    for line in lines {
        measured.push(measure_line(line, dpi, dots_per_point, scale)?);
    }
    Ok(measured)
}

fn measure_line(
    line: &[Run<'_>],
    dpi: i32,
    dots_per_point: f32,
    scale: f32,
) -> Result<MeasuredLine, PageError> {
    let mut glyphs = Vec::new();
    let mut cursor = 0.0f32;
    let mut line_h = 1.0f32;
    for run in line {
        let size = font_px(run.font_size * scale, dpi, dots_per_point);
        let font = open_font(run.families, run.weight, run.style)?;
        let metrics = font.horizontal_line_metrics(size);
        let ascent = metrics.map(|metrics| metrics.ascent).unwrap_or(size * 0.8);
        line_h = line_h.max(metrics.map(|metrics| metrics.new_line_size).unwrap_or(size));
        let color = rgb(run.color);
        for ch in run.text.chars() {
            let (glyph, bitmap) = font.rasterize(ch, size);
            let x = cursor + glyph.xmin as f32;
            let y = ascent - glyph.ymin as f32 - glyph.height as f32;
            if glyph.width > 0 && glyph.height > 0 {
                glyphs.push(Glyph {
                    x,
                    y,
                    width: glyph.width,
                    height: glyph.height,
                    bitmap,
                    color,
                });
            }
            cursor += glyph.advance_width;
        }
    }
    Ok(MeasuredLine {
        width: cursor,
        line_h,
        glyphs,
    })
}

fn blit(
    image: &mut RgbaImage,
    glyph: &Glyph,
    left: f32,
    top: f32,
    content_w: f32,
    content_h: f32,
    center_x: f32,
    center_y: f32,
    rotation: i32,
) {
    let width = image.width() as i32;
    let height = image.height() as i32;
    for row in 0..glyph.height {
        for col in 0..glyph.width {
            let coverage = glyph.bitmap[row * glyph.width + col];
            if coverage == 0 {
                continue;
            }
            let content_x = left + glyph.x + col as f32;
            let content_y = top + glyph.y + row as f32;
            let (rotated_x, rotated_y) = rotate(
                content_x - content_w / 2.0,
                content_y - content_h / 2.0,
                rotation,
            );
            let x = (center_x + rotated_x).round() as i32;
            let y = (center_y + rotated_y).round() as i32;
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let pixel = image.get_pixel_mut(x as u32, y as u32);
            blend(pixel, glyph.color, coverage);
        }
    }
}

fn rotate(x: f32, y: f32, rotation: i32) -> (f32, f32) {
    match rotation {
        1 => (-y, x),
        2 => (-x, -y),
        3 => (y, -x),
        _ => (x, y),
    }
}

fn blend(pixel: &mut Rgba<u8>, color: [u8; 3], coverage: u8) {
    let cover = coverage as u16;
    let keep = 255 - cover;
    for channel in 0..3 {
        pixel.0[channel] =
            ((color[channel] as u16 * cover + pixel.0[channel] as u16 * keep) / 255) as u8;
    }
    pixel.0[3] = 255;
}

fn rgb(color: i32) -> [u8; 3] {
    let color = color as u32;
    [(color >> 16) as u8, (color >> 8) as u8, color as u8]
}

fn open_font(families: &[String], weight: &str, style: &str) -> Result<Font, PageError> {
    let weight = css_weight(weight);
    let italic = css_italic(style);
    for family in families {
        if let Some(font) = font_file(Path::new(family)) {
            return Ok(font);
        }
        if let Some(font) = font_for_family(family, weight, italic) {
            return Ok(font);
        }
    }
    Font::from_bytes(TEST_FONT, FontSettings::default()).map_err(|message| err(message))
}

fn font_for_family(family: &str, weight: u16, italic: bool) -> Option<Font> {
    let slug = family_slug(family);
    if slug.is_empty() {
        return None;
    }
    let faces = list_faces(&Path::new("fonts").join(&slug), &slug);
    let path = choose_face(&faces, weight, italic)?;
    font_file(path)
}

struct FaceFile {
    weight: u16,
    italic: bool,
    otf: bool,
    path: PathBuf,
}

fn list_faces(dir: &Path, slug: &str) -> Vec<FaceFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut faces = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((weight, italic, otf)) = parse_face_name(slug, name) else {
            continue;
        };
        faces.push(FaceFile {
            weight,
            italic,
            otf,
            path: entry.path(),
        });
    }
    faces
}

fn parse_face_name(slug: &str, file_name: &str) -> Option<(u16, bool, bool)> {
    let (stem, ext) = file_name.rsplit_once('.')?;
    let otf = match ext.to_ascii_lowercase().as_str() {
        "otf" => true,
        "ttf" => false,
        _ => None?,
    };
    let rest = stem.strip_prefix(&format!("{slug}-"))?;
    let (weight, style) = rest.rsplit_once('-')?;
    let weight = weight.parse().ok()?;
    if !(1..=1000).contains(&weight) {
        return None;
    }
    let italic = match style {
        "italic" => true,
        "normal" => false,
        _ => return None,
    };
    Some((weight, italic, otf))
}

fn choose_face(faces: &[FaceFile], weight: u16, italic: bool) -> Option<&Path> {
    let styled: Vec<&FaceFile> = faces.iter().filter(|face| face.italic == italic).collect();
    let pool: Vec<&FaceFile> = if styled.is_empty() {
        faces.iter().filter(|face| !face.italic).collect()
    } else {
        styled
    };
    if pool.is_empty() {
        return None;
    }
    let weights: Vec<u16> = pool.iter().map(|face| face.weight).collect();
    let chosen = match_weight(&weights, weight);
    pool.iter()
        .filter(|face| face.weight == chosen)
        .max_by_key(|face| face.otf)
        .map(|face| face.path.as_path())
}

fn match_weight(weights: &[u16], desired: u16) -> u16 {
    if weights.contains(&desired) {
        return desired;
    }
    let mut below: Vec<u16> = weights
        .iter()
        .copied()
        .filter(|weight| *weight < desired)
        .collect();
    let mut above: Vec<u16> = weights
        .iter()
        .copied()
        .filter(|weight| *weight > desired)
        .collect();
    below.sort_unstable();
    above.sort_unstable();
    match desired {
        400 if weights.contains(&500) => 500,
        500 if weights.contains(&400) => 400,
        1..=399 => below
            .pop()
            .or_else(|| above.first().copied())
            .unwrap_or(desired),
        501..=1000 => above
            .first()
            .copied()
            .or_else(|| below.pop())
            .unwrap_or(desired),
        _ => nearest(weights, desired),
    }
}

fn nearest(weights: &[u16], desired: u16) -> u16 {
    weights
        .iter()
        .copied()
        .min_by_key(|weight| {
            let distance = weight.abs_diff(desired);
            let side = if desired > 500 {
                u16::MAX - *weight
            } else {
                *weight
            };
            (distance, side)
        })
        .unwrap_or(desired)
}

fn css_weight(weight: &str) -> u16 {
    match weight {
        "bold" => 700,
        "normal" | "" => 400,
        other => other.parse::<u16>().unwrap_or(400).clamp(1, 1000),
    }
}

fn css_italic(style: &str) -> bool {
    style == "italic" || style == "oblique"
}

fn family_slug(family: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for ch in family.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                slug.push(lower);
            }
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

fn font_file(path: &Path) -> Option<Font> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "ttf" && ext != "otf" {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Font::from_bytes(bytes, FontSettings::default()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Alignment, AreaKind, TextPart};

    fn layout(areas: Vec<Area>) -> Layout {
        Layout {
            width: 144,
            height: 72,
            dots_per_point: 1.0,
            areas,
        }
    }

    fn text_layout() -> Layout {
        layout(vec![Area {
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
            condition: "show".to_string(),
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
        }])
    }

    fn values(show: &str) -> HashMap<String, String> {
        HashMap::from([
            ("name".to_string(), "Ann".to_string()),
            ("show".to_string(), show.to_string()),
        ])
    }

    fn solid() -> Vec<u8> {
        let image = RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn decode(bytes: &[u8]) -> RgbaImage {
        image::load_from_memory(bytes).unwrap().to_rgba8()
    }

    #[test]
    fn otf_weight_is_selected() {
        let regular = open_font(&["Sample".to_string()], "normal", "normal").unwrap();
        let bold = open_font(&["Sample".to_string()], "bold", "normal").unwrap();
        let italic = open_font(&["Sample".to_string()], "400", "italic").unwrap();
        let regular_width = regular.rasterize('A', 100.0).0.advance_width;
        let bold_width = bold.rasterize('A', 100.0).0.advance_width;
        assert!(bold_width > regular_width);
        assert_eq!(italic.rasterize('A', 100.0).0.advance_width, regular_width);
    }

    #[test]
    fn png_has_the_page_size() {
        let png = render_png(&layout(Vec::new()), &HashMap::new(), 100, &solid(), None).unwrap();
        let image = decode(&png);
        assert_eq!(image.width(), 200);
        assert_eq!(image.height(), 100);
    }

    #[test]
    fn text_is_drawn() {
        let layout = text_layout();
        let png = render_png(&layout, &values("1"), 100, &solid(), None).unwrap();
        let image = decode(&png);
        let (start_x, start_y, end_x, end_y) =
            area_px(&layout.areas[0].rect, image.width(), image.height());
        assert!(
            changed(&image, start_x, start_y, end_x, end_y).is_some(),
            "text did not change a pixel inside the box"
        );
    }

    #[test]
    fn hidden_area_is_not_drawn() {
        let layout = text_layout();
        let shown = decode(&render_png(&layout, &values("1"), 100, &solid(), None).unwrap());
        let (start_x, start_y, end_x, end_y) =
            area_px(&layout.areas[0].rect, shown.width(), shown.height());
        let (x, y) = changed(&shown, start_x, start_y, end_x, end_y).expect("text pixel");
        let hidden = decode(&render_png(&layout, &values("false"), 100, &solid(), None).unwrap());
        assert_eq!(hidden.get_pixel(x, y).0, [255, 255, 255, 255]);
    }

    fn changed(
        image: &RgbaImage,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Option<(u32, u32)> {
        for y in start_y..end_y {
            for x in start_x..end_x {
                let pixel = image.get_pixel(x as u32, y as u32);
                if pixel.0 != [255, 255, 255, 255] {
                    return Some((x as u32, y as u32));
                }
            }
        }
        None
    }
}
