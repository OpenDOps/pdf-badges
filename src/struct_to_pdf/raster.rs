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
    Barcode, BoxFrame, ContentEntry, HorizontalAlign, ImagePlacement, Page, Photo, RenderError,
    Rgb, TextBox, Units, VerticalAlign,
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
    let mut canvas = Canvas {
        image: &mut image,
        ox: 0,
        oy: 0,
    };
    for entry in &page.contents {
        if entry.frame().is_plain() {
            draw_entry(page, entry, dpi, base_dir, &mut fonts, faces, &mut canvas)?;
        } else {
            draw_framed_entry(page, entry, dpi, base_dir, &mut fonts, faces, &mut canvas)?;
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

fn draw_entry(
    page: &Page,
    entry: &ContentEntry,
    dpi: f64,
    base_dir: &Path,
    fonts: &mut HashMap<String, Font>,
    faces: Option<&FaceSet>,
    image: &mut Canvas<'_>,
) -> Result<(), RenderError> {
    match entry {
        ContentEntry::Text(text) => draw_text(page, text, dpi, base_dir, fonts, faces, image),
        ContentEntry::Image(placement) => draw_resource(page, placement, dpi, base_dir, image),
        ContentEntry::Barcode(code) => draw_barcode(page, code, dpi, image),
        ContentEntry::Photo(photo) => draw_photo(page, photo, dpi, image),
    }
}

fn draw_framed_entry(
    page: &Page,
    entry: &ContentEntry,
    dpi: f64,
    base_dir: &Path,
    fonts: &mut HashMap<String, Font>,
    faces: Option<&FaceSet>,
    dest: &mut Canvas<'_>,
) -> Result<(), RenderError> {
    let frame = entry.frame();
    let (pos_x, pos_y, width, height) = entry.bounds();
    let (left, top, right, bottom) = layer_bounds(page, frame, pos_x, pos_y, width, height, dpi);
    if right <= left || bottom <= top {
        return Ok(());
    }
    let layer_w = (right - left) as u32;
    let layer_h = (bottom - top) as u32;
    let mut buffer = RgbaImage::new(layer_w, layer_h);
    {
        let mut layer = Canvas {
            image: &mut buffer,
            ox: left,
            oy: top,
        };
        fill_frame(&mut layer, page, frame, pos_x, pos_y, width, height, dpi);
        stroke_frame(&mut layer, page, frame, pos_x, pos_y, width, height, dpi);
        if frame.border_radius > 0.0 {
            let mut entry_buffer = RgbaImage::new(layer_w, layer_h);
            {
                let mut entry_layer = Canvas {
                    image: &mut entry_buffer,
                    ox: left,
                    oy: top,
                };
                draw_entry(page, entry, dpi, base_dir, fonts, faces, &mut entry_layer)?;
                clip_frame(
                    &mut entry_layer,
                    page,
                    frame,
                    pos_x,
                    pos_y,
                    width,
                    height,
                    dpi,
                );
            }
            blend_layer(&entry_buffer, left, top, &mut layer, 1.0);
        } else {
            draw_entry(page, entry, dpi, base_dir, fonts, faces, &mut layer)?;
        }
    }
    composite_frame(
        &buffer, left, top, dest, page, frame, pos_x, pos_y, width, height, dpi,
    );
    Ok(())
}

fn draw_barcode(
    page: &Page,
    code: &Barcode,
    dpi: f64,
    image: &mut Canvas<'_>,
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
    image: &mut Canvas<'_>,
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
    image: &mut Canvas<'_>,
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
    image: &mut Canvas<'_>,
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
            image.put(
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
    image: &mut Canvas<'_>,
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
    let tracking = length_px(text.text.letter_spacing, page.page_size.units, dpi) as f32;
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
            tracking,
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
            tracking,
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
                tracking,
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
                        image.blend(x, y, 255, segment.color);
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
    tracking: f32,
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
        tracking,
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
            tracking,
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
    tracking: f32,
) -> Result<Vec<PixLine>, RenderError> {
    let mut lines = Vec::new();
    let mut words = Vec::new();
    for item in items {
        match item {
            MarkupItem::Run(run) => {
                for piece in run.text.split(' ').filter(|word| !word.is_empty()) {
                    let word = word_from(fonts, faces, run, piece, units, dpi, id, tracking)?;
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
    tracking: f32,
) -> Result<PixWord, RenderError> {
    let size = length_px(run.font_size, units, dpi).max(1.0) as f32;
    let leading = length_px(run.leading, units, dpi).max(0.0) as f32;
    let line_height = length_px(run.line_height, units, dpi).max(1.0) as f32;
    let (font, glyphs) = face_for(fonts, faces, &run.font_key)?;
    let chars = text.chars().count() as f32;
    let width = measure(font, glyphs, text, size, id)? + tracking * chars;
    let space = measure(font, glyphs, " ", size, id)? + tracking;
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
    image: &mut Canvas<'_>,
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
    tracking: f32,
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
                image.blend(x, y, coverage, color);
            }
        }
        cursor += glyph.advance_width + tracking;
    }
}

fn paint_bars(
    image: &mut Canvas<'_>,
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
                image.put(x, y, Rgba([0, 0, 0, 255]));
            }
        }
    }
}

fn paint_matrix(
    image: &mut Canvas<'_>,
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
                    image.put(px, py, Rgba([0, 0, 0, 255]));
                }
            }
        }
    }
}

/// Page pixels written into a buffer that may be a crop of the page.
/// `ox` and `oy` are the page-pixel coordinate of the buffer's top-left.
struct Canvas<'a> {
    image: &'a mut RgbaImage,
    ox: i32,
    oy: i32,
}

impl Canvas<'_> {
    /// Straight source-over. `coverage` is the ink alpha. On an opaque pixel this
    /// matches a blend that writes alpha 255. On a transparent pixel the ink color
    /// is kept and `coverage` becomes the stored alpha.
    fn blend(&mut self, x: i32, y: i32, coverage: u8, color: Rgb) {
        if coverage == 0 {
            return;
        }
        let x = x - self.ox;
        let y = y - self.oy;
        if x < 0 || y < 0 {
            return;
        }
        let x = x as u32;
        let y = y as u32;
        if x >= self.image.width() || y >= self.image.height() {
            return;
        }
        let dst = *self.image.get_pixel(x, y);
        let src_a = u32::from(coverage);
        let dst_a = u32::from(dst[3]);
        let out_a = src_a + dst_a * (255 - src_a) / 255;
        if out_a == 0 {
            return;
        }
        let channel = |ink: u8, dst_c: u8| -> u8 {
            let pre = u32::from(ink) * src_a + u32::from(dst_c) * dst_a * (255 - src_a) / 255;
            (pre / out_a) as u8
        };
        self.image.put_pixel(
            x,
            y,
            Rgba([
                channel(color.red, dst[0]),
                channel(color.green, dst[1]),
                channel(color.blue, dst[2]),
                out_a as u8,
            ]),
        );
    }

    fn put(&mut self, x: i32, y: i32, color: Rgba<u8>) {
        let x = x - self.ox;
        let y = y - self.oy;
        if x < 0 || y < 0 {
            return;
        }
        let x = x as u32;
        let y = y as u32;
        if x < self.image.width() && y < self.image.height() {
            self.image.put_pixel(x, y, color);
        }
    }

    fn put_opaque(&mut self, x: i32, y: i32, color: Rgb) {
        self.put(x, y, Rgba([color.red, color.green, color.blue, 255]));
    }

    fn clear(&mut self, x: i32, y: i32) {
        self.put(x, y, Rgba([0, 0, 0, 0]));
    }
}

/// Pixels the frame actually paints: the box, plus half the border, plus one
/// pixel for rounding. Scale and skew are applied later, when this buffer is
/// copied, so the buffer stays the pre-transform rectangle.
fn layer_bounds(
    page: &Page,
    frame: &BoxFrame,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpi: f64,
) -> (i32, i32, i32, i32) {
    let half = frame
        .border
        .as_ref()
        .map(|border| border.width / 2.0)
        .unwrap_or(0.0);
    let (left, top, right, bottom) = entry_box(
        page,
        x - half,
        y - half,
        width + half * 2.0,
        height + half * 2.0,
        dpi,
    );
    (left - 1, top - 1, right + 1, bottom + 1)
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

fn fill_frame(
    image: &mut Canvas<'_>,
    page: &Page,
    frame: &BoxFrame,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpi: f64,
) {
    let Some(color) = frame.background else {
        return;
    };
    let (left, top, right, bottom) = entry_box(page, x, y, width, height, dpi);
    for py in top..bottom {
        for px in left..right {
            let (lx, ly) = local_units(page, x, y, px, py, dpi);
            if inside_round(lx, ly, width, height, frame.border_radius) {
                image.put_opaque(px, py, color);
            }
        }
    }
}

fn clip_frame(
    image: &mut Canvas<'_>,
    page: &Page,
    frame: &BoxFrame,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpi: f64,
) {
    if frame.border_radius <= 0.0 {
        return;
    }
    let (left, top, right, bottom) = entry_box(page, x, y, width, height, dpi);
    for py in top..bottom {
        for px in left..right {
            let (lx, ly) = local_units(page, x, y, px, py, dpi);
            if !inside_round(lx, ly, width, height, frame.border_radius) {
                image.clear(px, py);
            }
        }
    }
}

fn stroke_frame(
    image: &mut Canvas<'_>,
    page: &Page,
    frame: &BoxFrame,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpi: f64,
) {
    let Some(border) = &frame.border else {
        return;
    };
    let half = border.width / 2.0;
    let (left, top, right, bottom) = entry_box(
        page,
        x - half,
        y - half,
        width + border.width,
        height + border.width,
        dpi,
    );
    for py in top..bottom {
        for px in left..right {
            let (lx, ly) = local_units(page, x, y, px, py, dpi);
            let outer = inside_stroke_bounds(lx, ly, width, height, frame.border_radius, half);
            let inner = lx >= half
                && ly >= half
                && lx <= width - half
                && ly <= height - half
                && inside_corner(
                    lx,
                    ly,
                    width,
                    height,
                    frame.border_radius,
                    (frame.border_radius - half).max(0.0),
                );
            if outer && !inner {
                image.put_opaque(px, py, border.color);
            }
        }
    }
}

fn composite_frame(
    layer: &RgbaImage,
    ox: i32,
    oy: i32,
    dest: &mut Canvas<'_>,
    page: &Page,
    frame: &BoxFrame,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpi: f64,
) {
    let identity =
        frame.scale_x == 1.0 && frame.scale_y == 1.0 && frame.skew_x == 0.0 && frame.skew_y == 0.0;
    if identity {
        blend_layer(layer, ox, oy, dest, frame.opacity);
        return;
    }
    let half = frame
        .border
        .as_ref()
        .map(|border| border.width / 2.0)
        .unwrap_or(0.0);
    let corners = [
        (x - half, y - half),
        (x + width + half, y - half),
        (x - half, y + height + half),
        (x + width + half, y + height + half),
    ];
    let mapped: Vec<(f64, f64)> = corners
        .into_iter()
        .map(|(cx, cy)| map_document(page, frame, x, y, cx, cy))
        .collect();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (mx, my) in &mapped {
        min_x = min_x.min(*mx);
        min_y = min_y.min(*my);
        max_x = max_x.max(*mx);
        max_y = max_y.max(*my);
    }
    let (left, top, _, _) = entry_box(page, min_x, min_y, 0.0, 0.0, dpi);
    let (_, _, right, bottom) = entry_box(page, max_x, max_y, 0.0, 0.0, dpi);
    let opacity = opacity_coverage(frame.opacity);
    for py in (top - 1)..(bottom + 1) {
        for px in (left - 1)..(right + 1) {
            let (doc_x, doc_y) = pixel_to_document(page, px, py, dpi);
            let Some((src_x, src_y)) = unmap_document(page, frame, x, y, doc_x, doc_y) else {
                continue;
            };
            if !inside_stroke_bounds(
                src_x - x,
                src_y - y,
                width,
                height,
                frame.border_radius,
                half,
            ) {
                continue;
            }
            let (sx, sy) = document_to_pixel(page, src_x, src_y, dpi);
            let Some((color, alpha)) = layer_sample(layer, sx - ox, sy - oy) else {
                continue;
            };
            let coverage = scale_coverage(alpha, opacity);
            dest.blend(px, py, coverage, color);
        }
    }
}

fn blend_layer(layer: &RgbaImage, ox: i32, oy: i32, dest: &mut Canvas<'_>, opacity: f64) {
    let opacity = opacity_coverage(opacity);
    for (x, y, pixel) in layer.enumerate_pixels() {
        let coverage = scale_coverage(pixel[3], opacity);
        dest.blend(
            x as i32 + ox,
            y as i32 + oy,
            coverage,
            Rgb {
                red: pixel[0],
                green: pixel[1],
                blue: pixel[2],
            },
        );
    }
}

fn opacity_coverage(opacity: f64) -> u32 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u32
}

fn scale_coverage(alpha: u8, opacity: u32) -> u8 {
    (u32::from(alpha) * opacity / 255) as u8
}

/// The box plus the stroke's outer half. A sharp frame stays square. A rounded
/// frame uses the outer curve, so the centered stroke is not cut off.
fn inside_stroke_bounds(
    lx: f64,
    ly: f64,
    width: f64,
    height: f64,
    radius: f64,
    outset: f64,
) -> bool {
    let radius = if radius > 0.0 { radius + outset } else { 0.0 };
    inside_round(
        lx + outset,
        ly + outset,
        width + outset * 2.0,
        height + outset * 2.0,
        radius,
    )
}

fn inside_round(lx: f64, ly: f64, width: f64, height: f64, radius: f64) -> bool {
    inside_corner(lx, ly, width, height, radius, radius)
}

/// `zone` is the corner square. `circle` is the radius of the arc centered on
/// that square's inner corner. A stroke passes the outer radius as `zone` and
/// the inner radius as `circle`, so the band between the two arcs is outside.
fn inside_corner(lx: f64, ly: f64, width: f64, height: f64, zone: f64, circle: f64) -> bool {
    if width <= 0.0 || height <= 0.0 || lx < 0.0 || ly < 0.0 || lx > width || ly > height {
        return false;
    }
    if zone <= 0.0 {
        return true;
    }
    let zone = zone.min(width / 2.0).min(height / 2.0);
    let circle = circle.max(0.0);
    let corner = |cx: f64, cy: f64| {
        let dx = lx - cx;
        let dy = ly - cy;
        dx * dx + dy * dy <= circle * circle
    };
    if lx < zone && ly < zone {
        return corner(zone, zone);
    }
    if lx > width - zone && ly < zone {
        return corner(width - zone, zone);
    }
    if lx < zone && ly > height - zone {
        return corner(zone, height - zone);
    }
    if lx > width - zone && ly > height - zone {
        return corner(width - zone, height - zone);
    }
    true
}

fn local_units(
    page: &Page,
    origin_x: f64,
    origin_y: f64,
    px: i32,
    py: i32,
    dpi: f64,
) -> (f64, f64) {
    let (doc_x, doc_y) = pixel_to_document(page, px, py, dpi);
    (doc_x - origin_x, doc_y - origin_y)
}

fn pixel_to_document(page: &Page, px: i32, py: i32, dpi: f64) -> (f64, f64) {
    let inches_x = (f64::from(px) + 0.5) / dpi;
    let inches_y = (f64::from(py) + 0.5) / dpi;
    let value = |inches: f64| match page.page_size.units {
        Units::Points => inches * 72.0,
        Units::Mm => inches * 25.4,
    };
    (
        value(inches_x) - page.page_size.bleeds.left,
        value(inches_y) - page.page_size.bleeds.top,
    )
}

fn document_to_pixel(page: &Page, x: f64, y: f64, dpi: f64) -> (i32, i32) {
    let (left, top, _, _) = entry_box(page, x, y, 0.0, 0.0, dpi);
    (left, top)
}

fn layer_sample(layer: &RgbaImage, x: i32, y: i32) -> Option<(Rgb, u8)> {
    if x < 0 || y < 0 {
        return None;
    }
    let (x, y) = (x as u32, y as u32);
    if x >= layer.width() || y >= layer.height() {
        return None;
    }
    let pixel = layer.get_pixel(x, y);
    if pixel[3] == 0 {
        return None;
    }
    Some((
        Rgb {
            red: pixel[0],
            green: pixel[1],
            blue: pixel[2],
        },
        pixel[3],
    ))
}

fn map_document(
    page: &Page,
    frame: &BoxFrame,
    anchor_x: f64,
    anchor_y: f64,
    x: f64,
    y: f64,
) -> (f64, f64) {
    let (px, py) = crate::struct_to_pdf::pdf_point(page, x, y);
    let (ax, ay) = crate::struct_to_pdf::pdf_point(page, anchor_x, anchor_y);
    let matrix = crate::struct_to_pdf::geometry::frame_matrix(
        f64::from(ax),
        f64::from(ay),
        frame.scale_x,
        frame.scale_y,
        frame.skew_x,
        frame.skew_y,
    );
    let px2 = matrix[0] * f64::from(px) + matrix[2] * f64::from(py) + matrix[4];
    let py2 = matrix[1] * f64::from(px) + matrix[3] * f64::from(py) + matrix[5];
    pdf_to_document(page, px2, py2)
}

fn unmap_document(
    page: &Page,
    frame: &BoxFrame,
    anchor_x: f64,
    anchor_y: f64,
    x: f64,
    y: f64,
) -> Option<(f64, f64)> {
    let (px, py) = crate::struct_to_pdf::pdf_point(page, x, y);
    let (ax, ay) = crate::struct_to_pdf::pdf_point(page, anchor_x, anchor_y);
    let matrix = crate::struct_to_pdf::geometry::frame_matrix(
        f64::from(ax),
        f64::from(ay),
        frame.scale_x,
        frame.scale_y,
        frame.skew_x,
        frame.skew_y,
    );
    let det = matrix[0] * matrix[3] - matrix[2] * matrix[1];
    if det.abs() < 1e-9 {
        return None;
    }
    let px = f64::from(px) - matrix[4];
    let py = f64::from(py) - matrix[5];
    let src_x = (matrix[3] * px - matrix[2] * py) / det;
    let src_y = (-matrix[1] * px + matrix[0] * py) / det;
    Some(pdf_to_document(page, src_x, src_y))
}

fn pdf_to_document(page: &Page, px: f64, py: f64) -> (f64, f64) {
    let from = |points: f64| match page.page_size.units {
        Units::Points => points,
        Units::Mm => points * 25.4 / 72.0,
    };
    (
        from(px) - page.page_size.bleeds.left,
        page.page_size.height - (from(py) - page.page_size.bleeds.bottom),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        document_to_pixel, entry_box, length_px, local_units, map_document, paint, unmap_document,
    };
    use crate::struct_to_pdf::{load_page, Border, ContentEntry, Padding, Rgb};

    fn hello_page() -> (crate::struct_to_pdf::Page, PathBuf) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/struct-to-pdf/text-hello.yaml");
        let mut page = load_page(&path).expect("text-hello");
        let base_dir = path.parent().expect("fixture directory").to_path_buf();
        let ContentEntry::Text(text) = &mut page.contents[0] else {
            panic!("text box");
        };
        text.padding = Padding {
            top: 3.0,
            right: 3.0,
            bottom: 3.0,
            left: 3.0,
        };
        (page, base_dir)
    }

    #[test]
    fn framed_text_edge_matches_plain_text() {
        let (plain, base_dir) = hello_page();
        let mut framed = plain.clone();
        let ContentEntry::Text(text) = &mut framed.contents[0] else {
            unreachable!();
        };
        text.frame.border = Some(Border {
            width: 0.2,
            color: Rgb {
                red: 0,
                green: 0,
                blue: 255,
            },
        });
        let dpi = 144.0;
        let plain_img = paint(&plain, dpi, &base_dir, None).unwrap();
        let framed_img = paint(&framed, dpi, &base_dir, None).unwrap();
        let mut ink = 0u32;
        let mut worst = 0i32;
        for (left, right) in plain_img.pixels().zip(framed_img.pixels()) {
            let sum = i32::from(left[0]) + i32::from(left[1]) + i32::from(left[2]);
            if sum > 250 * 3 {
                continue;
            }
            if right[2] > right[0].saturating_add(40) {
                continue;
            }
            ink += 1;
            for channel in 0..3 {
                worst = worst.max((i32::from(left[channel]) - i32::from(right[channel])).abs());
            }
        }
        assert!(ink > 20, "expected antialiased ink, saw {ink}");
        assert!(
            worst <= 2,
            "framed edge drifted by {worst} across {ink} ink pixels"
        );
        let border = framed_img
            .pixels()
            .filter(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 255)
            .count();
        assert!(border > 10, "border was clipped, saw {border}");

        let ContentEntry::Text(text) = &mut framed.contents[0] else {
            unreachable!();
        };
        text.frame.scale_x = 1.5;
        let scaled = paint(&framed, dpi, &base_dir, None).unwrap();
        let gray = scaled
            .pixels()
            .filter(|pixel| {
                let spread = pixel[0]
                    .abs_diff(pixel[1])
                    .max(pixel[1].abs_diff(pixel[2]))
                    .max(pixel[0].abs_diff(pixel[2]));
                spread < 12 && (40..220).contains(&pixel[0])
            })
            .count();
        assert!(
            gray > 30,
            "scaled framed edge collapsed to a hard rim, gray pixels {gray}"
        );
    }

    #[test]
    fn framed_layer_covers_the_box_not_the_page() {
        use super::{entry_box, layer_bounds, page_px};

        let (mut page, _) = hello_page();
        let ContentEntry::Text(text) = &mut page.contents[0] else {
            panic!("text box");
        };
        text.frame.border = Some(Border {
            width: 2.0,
            color: Rgb {
                red: 0,
                green: 0,
                blue: 255,
            },
        });
        let text = text.clone();
        let dpi = 300.0;
        let (left, top, right, bottom) = layer_bounds(
            &page,
            &text.frame,
            text.pos_x,
            text.pos_y,
            text.width,
            text.height,
            dpi,
        );
        let page_w = page_px(
            page.page_size.width + page.page_size.bleeds.left + page.page_size.bleeds.right,
            page.page_size.units,
            dpi,
        );
        let page_h = page_px(
            page.page_size.height + page.page_size.bleeds.top + page.page_size.bleeds.bottom,
            page.page_size.units,
            dpi,
        );
        let layer_w = (right - left) as u32;
        let layer_h = (bottom - top) as u32;
        assert!(
            layer_w < page_w,
            "layer width {layer_w} covers the page {page_w}"
        );
        assert!(
            layer_h < page_h,
            "layer height {layer_h} covers the page {page_h}"
        );
        let (box_left, box_top, box_right, box_bottom) =
            entry_box(&page, text.pos_x, text.pos_y, text.width, text.height, dpi);
        assert!(left < box_left && top < box_top && right > box_right && bottom > box_bottom);

        let mut scaled = text.frame.clone();
        scaled.scale_x = 2.0;
        scaled.skew_x = 20.0;
        assert_eq!(
            (left, top, right, bottom),
            layer_bounds(
                &page,
                &scaled,
                text.pos_x,
                text.pos_y,
                text.width,
                text.height,
                dpi
            )
        );
    }

    #[test]
    fn border_sits_under_the_glyphs() {
        let (mut page, base_dir) = hello_page();
        let (pos_x, pos_y, width, height) = {
            let ContentEntry::Text(text) = &mut page.contents[0] else {
                panic!("text box");
            };
            text.padding = Padding {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            };
            text.frame.border = Some(Border {
                width: 8.0,
                color: Rgb {
                    red: 0,
                    green: 0,
                    blue: 255,
                },
            });
            (text.pos_x, text.pos_y, text.width, text.height)
        };
        let dpi = 144.0;
        let image = paint(&page, dpi, &base_dir, None).unwrap();
        let (left, top, right, _) = entry_box(&page, pos_x, pos_y, width, height, dpi);
        let y = top + length_px(1.5, page.page_size.units, dpi).round() as i32;
        let ink = (left..right)
            .filter(|&x| {
                let pixel = image.get_pixel(x as u32, y as u32);
                pixel[0] < 80 && pixel[2] < 200
            })
            .count();
        assert!(ink > 5, "border covered the glyphs, ink pixels {ink}");
    }

    #[test]
    fn scaled_border_keeps_its_outer_half() {
        let (mut page, base_dir) = hello_page();
        let (frame, pos_x, pos_y) = {
            let ContentEntry::Text(text) = &mut page.contents[0] else {
                panic!("text box");
            };
            text.frame.border = Some(Border {
                width: 4.0,
                color: Rgb {
                    red: 0,
                    green: 0,
                    blue: 255,
                },
            });
            text.frame.scale_x = 2.0;
            (text.frame.clone(), text.pos_x, text.pos_y)
        };
        let dpi = 144.0;
        let image = paint(&page, dpi, &base_dir, None).unwrap();
        let (doc_x, doc_y) = map_document(&page, &frame, pos_x, pos_y, pos_x - 1.0, pos_y + 4.0);
        let (px, py) = document_to_pixel(&page, doc_x, doc_y, dpi);
        let pixel = image.get_pixel(px as u32, py as u32);
        assert!(
            pixel[0] < 30 && pixel[1] < 30 && pixel[2] > 200,
            "outer stroke missing at {px},{py}: {pixel:?}"
        );
    }

    #[test]
    fn rounded_stroke_paints_its_corner_band() {
        let (mut page, base_dir) = hello_page();
        let (pos_x, pos_y, width, height) = {
            let ContentEntry::Text(text) = &mut page.contents[0] else {
                panic!("text box");
            };
            text.padding = Padding {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            };
            text.text.preentered = Some(String::new());
            text.frame.border_radius = 10.0;
            text.frame.border = Some(Border {
                width: 2.0,
                color: Rgb {
                    red: 0,
                    green: 0,
                    blue: 255,
                },
            });
            (text.pos_x, text.pos_y, text.width, text.height)
        };
        let dpi = 144.0;
        let image = paint(&page, dpi, &base_dir, None).unwrap();
        let (left, top, right, bottom) = entry_box(
            &page,
            pos_x - 1.0,
            pos_y - 1.0,
            width + 2.0,
            height + 2.0,
            dpi,
        );
        let mut band = 0;
        for py in top..bottom {
            for px in left..right {
                let (lx, ly) = local_units(&page, pos_x, pos_y, px, py, dpi);
                if lx > 10.0 || ly > 10.0 {
                    continue;
                }
                let dist = ((lx - 10.0).powi(2) + (ly - 10.0).powi(2)).sqrt();
                if !(9.2..9.8).contains(&dist) {
                    continue;
                }
                let pixel = image.get_pixel(px as u32, py as u32);
                assert!(
                    pixel[0] < 30 && pixel[1] < 30 && pixel[2] > 200,
                    "corner band missing at {px},{py} dist {dist}: {pixel:?}"
                );
                band += 1;
            }
        }
        assert!(band > 3, "expected corner-band pixels, saw {band}");
    }

    #[test]
    fn positive_skew_x_moves_the_bottom_right() {
        let (page, _) = hello_page();
        let ContentEntry::Text(text) = &page.contents[0] else {
            panic!("text box");
        };
        let mut frame = text.frame.clone();
        frame.skew_x = 45.0;
        let bottom_y = text.pos_y + text.height;
        let (x, y) = map_document(&page, &frame, text.pos_x, text.pos_y, text.pos_x, bottom_y);
        assert!(
            x > text.pos_x + text.height - 0.5,
            "bottom moved to {x}, anchor {}",
            text.pos_x
        );
        assert!((y - bottom_y).abs() < 0.05, "bottom y moved to {y}");
        let (top_x, top_y) = map_document(
            &page, &frame, text.pos_x, text.pos_y, text.pos_x, text.pos_y,
        );
        assert!((top_x - text.pos_x).abs() < 0.01);
        assert!((top_y - text.pos_y).abs() < 0.01);
        let (back_x, back_y) =
            unmap_document(&page, &frame, text.pos_x, text.pos_y, x, y).expect("inverse");
        assert!((back_x - text.pos_x).abs() < 0.05);
        assert!((back_y - bottom_y).abs() < 0.05);
    }
}
