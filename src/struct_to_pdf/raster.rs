use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Mutex;

use fontdue::{Font, FontSettings, Metrics};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbImage, Rgba, RgbaImage};

use crate::struct_to_pdf::barcode::{self, Symbol};
use crate::struct_to_pdf::fonts;
use crate::struct_to_pdf::markup::{self, LineMetrics, MarkupItem, StyledRun};
use crate::struct_to_pdf::{
    Barcode, ContentEntry, HorizontalAlign, ImagePlacement, Page, Photo, RenderError, Rgb, TextBox,
    Units, VerticalAlign,
};

pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
    pub bmp: Vec<u8>,
}

pub(crate) struct FaceSet {
    faces: HashMap<String, CachedFace>,
}

struct CachedFace {
    font: Font,
    glyphs: Mutex<HashMap<(u32, char), (Metrics, Vec<u8>)>>,
}

impl FaceSet {
    pub(crate) fn from_page(page: &Page) -> Self {
        let mut faces = HashMap::new();
        for (key, resource) in &page.fonts {
            let Some(bytes) = resource.bytes.clone() else {
                continue;
            };
            let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) else {
                continue;
            };
            faces.insert(
                key.clone(),
                CachedFace {
                    font,
                    glyphs: Mutex::new(HashMap::new()),
                },
            );
        }
        FaceSet { faces }
    }

    fn get(&self, key: &str) -> Option<&CachedFace> {
        self.faces.get(key)
    }
}

pub(crate) fn render_pixels(
    page: &Page,
    dpi: f64,
    base_dir: &Path,
    faces: &FaceSet,
) -> Result<RgbaImage, RenderError> {
    paint(page, dpi, base_dir, Some(faces))
}

pub(crate) fn png_bytes(image: RgbaImage) -> Result<Vec<u8>, RenderError> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|err| RenderError {
            context: "page".to_string(),
            message: err.to_string(),
        })?;
    Ok(bytes)
}

pub(crate) fn bmp_bytes(image: RgbaImage) -> Result<Vec<u8>, RenderError> {
    let rgb = DynamicImage::ImageRgba8(image).into_rgb8();
    encode_bmp(&rgb)
}

pub fn render(page: &Page, dpi: f64, base_dir: &Path) -> Result<Raster, RenderError> {
    if dpi <= 0.0 {
        return Err(RenderError {
            context: "page".to_string(),
            message: "dpi must be positive".to_string(),
        });
    }
    let width = page_px(
        page.page_size.width + page.page_size.bleeds.left + page.page_size.bleeds.right,
        page.page_size.units,
        dpi,
    );
    let height = page_px(
        page.page_size.height + page.page_size.bleeds.top + page.page_size.bleeds.bottom,
        page.page_size.units,
        dpi,
    );
    if width == 0 || height == 0 {
        return Err(RenderError {
            context: "page".to_string(),
            message: "page has no pixels".to_string(),
        });
    }
    let image = paint(page, dpi, base_dir, None)?;
    Ok(Raster {
        width,
        height,
        png: encode_png(&image)?,
        bmp: encode_bmp(&DynamicImage::ImageRgba8(image).to_rgb8())?,
    })
}

fn paint(
    page: &Page,
    dpi: f64,
    base_dir: &Path,
    faces: Option<&FaceSet>,
) -> Result<RgbaImage, RenderError> {
    if dpi <= 0.0 {
        return Err(RenderError {
            context: "page".to_string(),
            message: "dpi must be positive".to_string(),
        });
    }
    let width = page_px(
        page.page_size.width + page.page_size.bleeds.left + page.page_size.bleeds.right,
        page.page_size.units,
        dpi,
    );
    let height = page_px(
        page.page_size.height + page.page_size.bleeds.top + page.page_size.bleeds.bottom,
        page.page_size.units,
        dpi,
    );
    if width == 0 || height == 0 {
        return Err(RenderError {
            context: "page".to_string(),
            message: "page has no pixels".to_string(),
        });
    }
    let background = page.page_size.background.unwrap_or(Rgb::WHITE);
    let mut image = RgbaImage::from_pixel(
        width,
        height,
        Rgba([background.red, background.green, background.blue, 255]),
    );
    let mut fonts = HashMap::new();
    for entry in &page.contents {
        match entry {
            ContentEntry::Text(text) => {
                draw_text(page, text, dpi, base_dir, &mut fonts, faces, &mut image)?
            }
            ContentEntry::Image(placement) => {
                draw_resource(page, placement, dpi, base_dir, &mut image)?
            }
            ContentEntry::Barcode(code) => draw_barcode(page, code, dpi, &mut image)?,
            ContentEntry::Photo(photo) => draw_photo(page, photo, dpi, &mut image)?,
        }
    }
    Ok(image)
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, RenderError> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|err| RenderError {
            context: "page".to_string(),
            message: err.to_string(),
        })?;
    Ok(bytes)
}

fn encode_bmp(image: &RgbImage) -> Result<Vec<u8>, RenderError> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(image.clone())
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Bmp)
        .map_err(|err| RenderError {
            context: "page".to_string(),
            message: err.to_string(),
        })?;
    Ok(bytes)
}

fn draw_barcode(
    page: &Page,
    code: &Barcode,
    dpi: f64,
    image: &mut RgbaImage,
) -> Result<(), RenderError> {
    let payload = code
        .content
        .clone()
        .or_else(|| code.preentered.clone())
        .unwrap_or_default();
    let (start_x, start_y, end_x, end_y) =
        entry_box(page, code.pos_x, code.pos_y, code.width, code.height, dpi);
    match barcode::symbol(&code.barcode_type, &payload, &code.id)? {
        Symbol::Bars(modules) => paint_bars(image, start_x, start_y, end_x, end_y, &modules),
        Symbol::Qr { width, modules } => {
            paint_matrix(image, start_x, start_y, end_x, end_y, width, &modules);
        }
    }
    Ok(())
}

fn draw_photo(
    page: &Page,
    photo: &Photo,
    dpi: f64,
    image: &mut RgbaImage,
) -> Result<(), RenderError> {
    let Some(bytes) = photo.bytes.as_deref().filter(|bytes| !bytes.is_empty()) else {
        return Ok(());
    };
    let (start_x, start_y, end_x, end_y) = entry_box(
        page,
        photo.pos_x,
        photo.pos_y,
        photo.width,
        photo.height,
        dpi,
    );
    blit_image(image, start_x, start_y, end_x, end_y, bytes, &photo.id)
}

fn draw_resource(
    page: &Page,
    placement: &ImagePlacement,
    dpi: f64,
    base_dir: &Path,
    image: &mut RgbaImage,
) -> Result<(), RenderError> {
    let resource = page
        .images
        .get(&placement.resource_name)
        .ok_or_else(|| RenderError {
            context: placement.id.clone(),
            message: format!("unknown image resource {}", placement.resource_name),
        })?;
    let bytes = if let Some(bytes) = resource.bytes.clone() {
        bytes
    } else {
        let relative = resource
            .extracted_path
            .as_deref()
            .ok_or_else(|| RenderError {
                context: placement.id.clone(),
                message: "extracted_path is missing".to_string(),
            })?;
        let path = base_dir.join(relative);
        std::fs::read(&path).map_err(|err| RenderError {
            context: placement.id.clone(),
            message: format!("{}: {err}", path.display()),
        })?
    };
    let (start_x, start_y, end_x, end_y) = entry_box(
        page,
        placement.pos_x,
        placement.pos_y,
        placement.width,
        placement.height,
        dpi,
    );
    blit_image(image, start_x, start_y, end_x, end_y, &bytes, &placement.id)
}

fn blit_image(
    image: &mut RgbaImage,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    bytes: &[u8],
    id: &str,
) -> Result<(), RenderError> {
    if end_x <= start_x || end_y <= start_y {
        return Ok(());
    }
    let source = image::load_from_memory(bytes).map_err(|err| RenderError {
        context: id.to_string(),
        message: err.to_string(),
    })?;
    let width = (end_x - start_x) as u32;
    let height = (end_y - start_y) as u32;
    let scaled = image::imageops::resize(&source.to_rgba8(), width, height, FilterType::Nearest);
    for y in 0..height {
        for x in 0..width {
            put(
                image,
                start_x + x as i32,
                start_y + y as i32,
                *scaled.get_pixel(x, y),
            );
        }
    }
    Ok(())
}

fn draw_text(
    page: &Page,
    text: &TextBox,
    dpi: f64,
    base_dir: &Path,
    fonts: &mut HashMap<String, Font>,
    faces: Option<&FaceSet>,
    image: &mut RgbaImage,
) -> Result<(), RenderError> {
    let shown = text
        .text
        .content
        .as_deref()
        .or(text.text.preentered.as_deref())
        .unwrap_or("");
    if shown.is_empty() {
        return Ok(());
    }
    let mut parsed = markup::parse(shown, text)?;
    ensure_fonts(page, text, base_dir, fonts, faces, &parsed)?;
    let (box_x, box_y, box_right, box_bottom) =
        entry_box(page, text.pos_x, text.pos_y, text.width, text.height, dpi);
    let pad_left = length_px(text.padding.left, page.page_size.units, dpi).round() as i32;
    let pad_top = length_px(text.padding.top, page.page_size.units, dpi).round() as i32;
    let pad_right = length_px(text.padding.right, page.page_size.units, dpi).round() as i32;
    let pad_bottom = length_px(text.padding.bottom, page.page_size.units, dpi).round() as i32;
    let origin_x = box_x + pad_left;
    let origin_y = box_y + pad_top;
    let content_w = (box_right - pad_right - origin_x).max(0);
    let content_h = (box_bottom - pad_bottom - origin_y).max(0);
    if content_w == 0 || content_h == 0 {
        return Ok(());
    }
    let units = page.page_size.units;
    let lines = if text.auto_scale {
        fit_lines(
            fonts,
            faces,
            &mut parsed,
            content_w as f32,
            content_h as f32,
            units,
            dpi,
            &text.id,
        )?
    } else {
        layout_lines(
            fonts,
            faces,
            &parsed.items,
            &parsed.end_metrics,
            content_w as f32,
            units,
            dpi,
            &text.id,
        )?
    };
    if lines.is_empty() {
        return Ok(());
    }
    let line_box_h: f32 = lines
        .iter()
        .map(|line| line.line_height + line.leading)
        .sum();
    let painted_h = painted_block_height_px(fonts, faces, &lines)?;
    let block_h = line_box_h.max(painted_h);
    let top = match text.vertical {
        VerticalAlign::Top => 0.0,
        VerticalAlign::Middle => ((content_h as f32) - block_h) / 2.0,
        VerticalAlign::Bottom => (content_h as f32) - block_h,
    };
    let mut cursor_y = top;
    for line in &lines {
        let left = match text.horizontal {
            HorizontalAlign::Left => 0.0,
            HorizontalAlign::Center => (content_w as f32 - line.width) / 2.0,
            HorizontalAlign::Right => content_w as f32 - line.width,
        };
        let mut cursor_x = left;
        for segment in &line.segments {
            let (font, glyphs) = face_for(fonts, faces, &segment.font_key)?;
            blit_line(
                image,
                font,
                glyphs,
                &segment.text,
                segment.size,
                segment.color,
                origin_x as f32 + cursor_x,
                origin_y as f32 + cursor_y,
                origin_x,
                origin_y,
                origin_x + content_w,
                origin_y + content_h,
            );
            if segment.underline && segment.width > 0.0 {
                let ascent = font
                    .horizontal_line_metrics(segment.size)
                    .map(|metrics| metrics.ascent)
                    .unwrap_or(segment.size * 0.8);
                let y = (origin_y as f32 + cursor_y + ascent + segment.size * 0.08).round() as i32;
                let x0 = (origin_x as f32 + cursor_x).round() as i32;
                let x1 = (origin_x as f32 + cursor_x + segment.width).round() as i32;
                for x in x0..x1 {
                    if x >= origin_x
                        && x < origin_x + content_w
                        && y >= origin_y
                        && y < origin_y + content_h
                    {
                        blend(image, x, y, 255, segment.color);
                    }
                }
            }
            cursor_x += segment.width;
        }
        cursor_y += line.line_height + line.leading;
    }
    Ok(())
}

struct PixSegment {
    text: String,
    font_key: String,
    size: f32,
    underline: bool,
    color: Rgb,
    width: f32,
}

struct PixLine {
    segments: Vec<PixSegment>,
    width: f32,
    line_height: f32,
    leading: f32,
}

struct PixWord {
    text: String,
    font_key: String,
    size: f32,
    leading: f32,
    line_height: f32,
    underline: bool,
    color: Rgb,
    width: f32,
    space: f32,
}

fn fit_lines(
    fonts: &HashMap<String, Font>,
    faces: Option<&FaceSet>,
    parsed: &mut markup::Markup,
    content_w: f32,
    content_h: f32,
    units: Units,
    dpi: f64,
    id: &str,
) -> Result<Vec<PixLine>, RenderError> {
    let mut lines = layout_lines(
        fonts,
        faces,
        &parsed.items,
        &parsed.end_metrics,
        content_w,
        units,
        dpi,
        id,
    )?;
    if lines.is_empty() {
        return Ok(lines);
    }
    for _ in 0..32 {
        let scale = fit_scale(fonts, faces, &lines, content_w, content_h)?;
        if scale >= 1.0 {
            break;
        }
        scale_markup(parsed, scale as f64);
        lines = layout_lines(
            fonts,
            faces,
            &parsed.items,
            &parsed.end_metrics,
            content_w,
            units,
            dpi,
            id,
        )?;
    }
    Ok(lines)
}

fn fit_scale(
    fonts: &HashMap<String, Font>,
    faces: Option<&FaceSet>,
    lines: &[PixLine],
    content_w: f32,
    content_h: f32,
) -> Result<f32, RenderError> {
    let text_w = lines.iter().map(|line| line.width).fold(0.0f32, f32::max);
    let line_box_h: f32 = lines
        .iter()
        .map(|line| line.line_height + line.leading)
        .sum();
    let text_h = line_box_h.max(painted_block_height_px(fonts, faces, lines)?);
    if text_w <= content_w + 0.01 && text_h <= content_h + 0.01 {
        return Ok(1.0);
    }
    let width_scale = if text_w > 0.0 {
        content_w / text_w
    } else {
        1.0
    };
    let height_scale = if text_h > 0.0 {
        content_h / text_h
    } else {
        1.0
    };
    Ok(width_scale.min(height_scale).min(1.0))
}

fn painted_block_height_px(
    fonts: &HashMap<String, Font>,
    faces: Option<&FaceSet>,
    lines: &[PixLine],
) -> Result<f32, RenderError> {
    if lines.is_empty() {
        return Ok(0.0);
    }
    // Raster positions each line from its top and draws with ascent inside the
    // line. The ink of the last line therefore needs ascent+descent below that
    // line's top, while earlier lines advance by line_height+leading.
    let steps: f32 = lines[..lines.len() - 1]
        .iter()
        .map(|line| line.line_height + line.leading)
        .sum();
    let last = lines.last().expect("line");
    let (ascent, descent) = if let Some(segment) = last.segments.first() {
        let (font, _) = face_for(fonts, faces, &segment.font_key)?;
        let metrics = font.horizontal_line_metrics(segment.size);
        (
            metrics.map(|m| m.ascent).unwrap_or(segment.size * 0.8),
            metrics
                .map(|m| m.descent.abs())
                .unwrap_or(segment.size * 0.2),
        )
    } else {
        (last.line_height * 0.8, last.line_height * 0.2)
    };
    Ok(steps + ascent + descent)
}

fn scale_markup(parsed: &mut markup::Markup, scale: f64) {
    for item in &mut parsed.items {
        match item {
            MarkupItem::Run(run) => {
                run.font_size *= scale;
                run.leading *= scale;
                run.line_height *= scale;
            }
            MarkupItem::Break(metrics) => scale_metrics(metrics, scale),
        }
    }
    scale_metrics(&mut parsed.end_metrics, scale);
}

fn scale_metrics(metrics: &mut LineMetrics, scale: f64) {
    metrics.font_size *= scale;
    metrics.leading *= scale;
    metrics.line_height *= scale;
}

fn layout_lines(
    fonts: &HashMap<String, Font>,
    faces: Option<&FaceSet>,
    items: &[MarkupItem],
    end_metrics: &LineMetrics,
    content_w: f32,
    units: Units,
    dpi: f64,
    id: &str,
) -> Result<Vec<PixLine>, RenderError> {
    let mut lines = Vec::new();
    let mut words = Vec::new();
    for item in items {
        match item {
            MarkupItem::Run(run) => {
                for piece in run.text.split(' ').filter(|word| !word.is_empty()) {
                    let word = word_from(fonts, faces, run, piece, units, dpi, id)?;
                    if words.is_empty()
                        || words_width(&words) + words.last().expect("word").space + word.width
                            <= content_w + 0.01
                    {
                        words.push(word);
                        continue;
                    }
                    let ended = words.last().expect("line has a word");
                    let line_height = ended.line_height;
                    let leading = ended.leading;
                    lines.push(finish_line(
                        std::mem::take(&mut words),
                        line_height,
                        leading,
                    ));
                    words.push(word);
                }
            }
            MarkupItem::Break(metrics) => {
                let line_height = length_px(metrics.line_height, units, dpi).max(1.0) as f32;
                let leading = length_px(metrics.leading, units, dpi).max(0.0) as f32;
                lines.push(finish_line(
                    std::mem::take(&mut words),
                    line_height,
                    leading,
                ));
            }
        }
    }
    if !words.is_empty() {
        let line_height = length_px(end_metrics.line_height, units, dpi).max(1.0) as f32;
        let leading = length_px(end_metrics.leading, units, dpi).max(0.0) as f32;
        lines.push(finish_line(words, line_height, leading));
    }
    Ok(lines)
}

fn word_from(
    fonts: &HashMap<String, Font>,
    faces: Option<&FaceSet>,
    run: &StyledRun,
    text: &str,
    units: Units,
    dpi: f64,
    id: &str,
) -> Result<PixWord, RenderError> {
    let size = length_px(run.font_size, units, dpi).max(1.0) as f32;
    let leading = length_px(run.leading, units, dpi).max(0.0) as f32;
    let line_height = length_px(run.line_height, units, dpi).max(1.0) as f32;
    let (font, glyphs) = face_for(fonts, faces, &run.font_key)?;
    let width = measure(font, glyphs, text, size, id)?;
    let space = measure(font, glyphs, " ", size, id)?;
    Ok(PixWord {
        text: text.to_string(),
        font_key: run.font_key.clone(),
        size,
        leading,
        line_height,
        underline: run.underline,
        color: run.color,
        width,
        space,
    })
}

fn finish_line(words: Vec<PixWord>, line_height: f32, leading: f32) -> PixLine {
    let (segments, width) = segments_of(&words);
    PixLine {
        segments,
        width,
        line_height,
        leading,
    }
}

fn words_width(words: &[PixWord]) -> f32 {
    let mut total = 0.0;
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            total += words[index - 1].space;
        }
        total += word.width;
    }
    total
}

fn segments_of(words: &[PixWord]) -> (Vec<PixSegment>, f32) {
    let mut segments: Vec<PixSegment> = Vec::new();
    let mut pending_space = 0.0;
    for word in words {
        if let Some(last) = segments.last_mut() {
            if last.font_key == word.font_key
                && (last.size - word.size).abs() < f32::EPSILON
                && last.underline == word.underline
                && last.color == word.color
            {
                last.text.push(' ');
                last.text.push_str(&word.text);
                last.width += pending_space + word.width;
                pending_space = word.space;
                continue;
            }
            last.text.push(' ');
            last.width += pending_space;
        }
        segments.push(PixSegment {
            text: word.text.clone(),
            font_key: word.font_key.clone(),
            size: word.size,
            underline: word.underline,
            color: word.color,
            width: word.width,
        });
        pending_space = word.space;
    }
    let width = segments.iter().map(|segment| segment.width).sum();
    (segments, width)
}

fn measure(
    font: &Font,
    glyphs: Option<&GlyphCache>,
    text: &str,
    size: f32,
    id: &str,
) -> Result<f32, RenderError> {
    let mut missing = Vec::new();
    let mut width = 0.0f32;
    for ch in text.chars() {
        if font.lookup_glyph_index(ch) == 0 && ch != ' ' {
            if !missing.contains(&ch) {
                missing.push(ch);
            }
            continue;
        }
        width += rasterize(font, glyphs, ch, size).0.advance_width;
    }
    if !missing.is_empty() {
        return Err(fonts::uncovered(id, &missing));
    }
    Ok(width)
}

fn ensure_fonts(
    page: &Page,
    text: &TextBox,
    base_dir: &Path,
    fonts: &mut HashMap<String, Font>,
    faces: Option<&FaceSet>,
    parsed: &markup::Markup,
) -> Result<(), RenderError> {
    let mut keys = Vec::new();
    for item in &parsed.items {
        match item {
            MarkupItem::Run(run) => push_key(&mut keys, &run.font_key),
            MarkupItem::Break(metrics) => push_key(&mut keys, &metrics.font_key),
        }
    }
    push_key(&mut keys, &parsed.end_metrics.font_key);
    for key in keys {
        if faces.is_some_and(|faces| faces.get(&key).is_some()) {
            continue;
        }
        open_font(page, &key, base_dir, fonts, &text.id)?;
    }
    Ok(())
}

fn push_key(keys: &mut Vec<String>, key: &str) {
    if !keys.iter().any(|existing| existing == key) {
        keys.push(key.to_string());
    }
}

fn face_for<'a>(
    fonts: &'a HashMap<String, Font>,
    faces: Option<&'a FaceSet>,
    key: &str,
) -> Result<(&'a Font, Option<&'a GlyphCache>), RenderError> {
    if let Some(face) = faces.and_then(|faces| faces.get(key)) {
        return Ok((&face.font, Some(&face.glyphs)));
    }
    let font = fonts.get(key).ok_or_else(|| RenderError {
        context: key.to_string(),
        message: format!("font {key} was not opened"),
    })?;
    Ok((font, None))
}

fn open_font<'a>(
    page: &Page,
    key: &str,
    base_dir: &Path,
    fonts: &'a mut HashMap<String, Font>,
    id: &str,
) -> Result<&'a Font, RenderError> {
    if !fonts.contains_key(key) {
        let resource = page.fonts.get(key).ok_or_else(|| RenderError {
            context: id.to_string(),
            message: format!("unknown font {key}"),
        })?;
        let bytes = if let Some(bytes) = resource.bytes.clone() {
            bytes
        } else {
            let relative = resource.source_path.as_deref().ok_or_else(|| RenderError {
                context: id.to_string(),
                message: "source_path is missing".to_string(),
            })?;
            let path = base_dir.join(relative);
            std::fs::read(&path).map_err(|_| RenderError {
                context: id.to_string(),
                message: format!("font file not found: {}", path.display()),
            })?
        };
        let font = Font::from_bytes(bytes, FontSettings::default()).map_err(|err| RenderError {
            context: id.to_string(),
            message: err.to_string(),
        })?;
        fonts.insert(key.to_string(), font);
    }
    Ok(fonts.get(key).expect("font was inserted"))
}

type GlyphCache = Mutex<HashMap<(u32, char), (Metrics, Vec<u8>)>>;

fn rasterize(font: &Font, glyphs: Option<&GlyphCache>, ch: char, size: f32) -> (Metrics, Vec<u8>) {
    let Some(glyphs) = glyphs else {
        return font.rasterize(ch, size);
    };
    let key = (size.to_bits(), ch);
    let mut cache = glyphs.lock().unwrap_or_else(|err| err.into_inner());
    if let Some((metrics, bitmap)) = cache.get(&key) {
        return (*metrics, bitmap.clone());
    }
    let (metrics, bitmap) = font.rasterize(ch, size);
    cache.insert(key, (metrics, bitmap.clone()));
    (metrics, bitmap)
}

fn blit_line(
    image: &mut RgbaImage,
    font: &Font,
    glyphs: Option<&GlyphCache>,
    text: &str,
    size: f32,
    color: Rgb,
    left: f32,
    top: f32,
    clip_left: i32,
    clip_top: i32,
    clip_right: i32,
    clip_bottom: i32,
) {
    let metrics = font.horizontal_line_metrics(size);
    let ascent = metrics.map(|metrics| metrics.ascent).unwrap_or(size * 0.8);
    let mut cursor = left;
    for ch in text.chars() {
        let (glyph, bitmap) = rasterize(font, glyphs, ch, size);
        let glyph_x = cursor + glyph.xmin as f32;
        let glyph_y = top + ascent - glyph.ymin as f32 - glyph.height as f32;
        for row in 0..glyph.height {
            for col in 0..glyph.width {
                let coverage = bitmap[row * glyph.width + col];
                if coverage == 0 {
                    continue;
                }
                let x = (glyph_x + col as f32).round() as i32;
                let y = (glyph_y + row as f32).round() as i32;
                if x < clip_left || y < clip_top || x >= clip_right || y >= clip_bottom {
                    continue;
                }
                blend(image, x, y, coverage, color);
            }
        }
        cursor += glyph.advance_width;
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
    if span == 0 || count == 0 || end_y <= start_y {
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
                put(image, x, y, Rgba([0, 0, 0, 255]));
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
    if width == 0 || end_x <= start_x || end_y <= start_y {
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
                    put(image, px, py, Rgba([0, 0, 0, 255]));
                }
            }
        }
    }
}

fn blend(image: &mut RgbaImage, x: i32, y: i32, coverage: u8, color: Rgb) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as u32;
    let y = y as u32;
    if x >= image.width() || y >= image.height() {
        return;
    }
    let pixel = image.get_pixel(x, y);
    let cover = u16::from(coverage);
    let keep = 255 - cover;
    let mix =
        |channel: u8, ink: u8| ((u16::from(ink) * cover + u16::from(channel) * keep) / 255) as u8;
    image.put_pixel(
        x,
        y,
        Rgba([
            mix(pixel[0], color.red),
            mix(pixel[1], color.green),
            mix(pixel[2], color.blue),
            255,
        ]),
    );
}

fn put(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as u32;
    let y = y as u32;
    if x < image.width() && y < image.height() {
        image.put_pixel(x, y, color);
    }
}

fn entry_box(
    page: &Page,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpi: f64,
) -> (i32, i32, i32, i32) {
    let units = page.page_size.units;
    let left = length_px(page.page_size.bleeds.left + x, units, dpi).round() as i32;
    let top = length_px(page.page_size.bleeds.top + y, units, dpi).round() as i32;
    let right = length_px(page.page_size.bleeds.left + x + width, units, dpi).round() as i32;
    let bottom = length_px(page.page_size.bleeds.top + y + height, units, dpi).round() as i32;
    (left, top, right, bottom)
}

fn page_px(value: f64, units: Units, dpi: f64) -> u32 {
    length_px(value, units, dpi).round().max(0.0) as u32
}

fn length_px(value: f64, units: Units, dpi: f64) -> f64 {
    let inches = match units {
        Units::Points => value / 72.0,
        Units::Mm => value / 25.4,
    };
    inches * dpi
}
