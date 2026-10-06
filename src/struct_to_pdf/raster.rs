use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Mutex;

use fontdue::{Font, FontSettings, Metrics};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbImage, Rgba, RgbaImage};

use crate::struct_to_pdf::barcode::{self, Symbol};
use crate::struct_to_pdf::{
    Barcode, ContentEntry, HorizontalAlign, ImagePlacement, Page, Photo, RenderError, TextBox,
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
    let mut image = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
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
    let cached = faces.and_then(|faces| faces.get(&text.text.font));
    let owned;
    let (font, glyphs) = if let Some(face) = cached {
        (&face.font, Some(&face.glyphs))
    } else {
        owned = open_font(page, &text.text.font, base_dir, fonts, &text.id)?;
        (owned, None)
    };
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
    let size = length_px(text.text.font_size, page.page_size.units, dpi).max(1.0) as f32;
    let line_h = length_px(
        text.text.line_height + text.text.leading,
        page.page_size.units,
        dpi,
    )
    .max(1.0);
    let lines = wrap(font, glyphs, shown, size, content_w as f32);
    let block_h = line_h * lines.len() as f64;
    let top = match text.vertical {
        VerticalAlign::Top => 0.0,
        VerticalAlign::Middle => ((content_h as f64) - block_h) / 2.0,
        VerticalAlign::Bottom => (content_h as f64) - block_h,
    };
    for (index, line) in lines.iter().enumerate() {
        let width = line_width(font, glyphs, line, size);
        let left = match text.horizontal {
            HorizontalAlign::Left => 0.0,
            HorizontalAlign::Center => (content_w as f32 - width) / 2.0,
            HorizontalAlign::Right => content_w as f32 - width,
        };
        let baseline = top + line_h * index as f64;
        blit_line(
            image,
            font,
            glyphs,
            line,
            size,
            origin_x as f32 + left,
            origin_y as f32 + baseline as f32,
            origin_x,
            origin_y,
            origin_x + content_w,
            origin_y + content_h,
        );
    }
    Ok(())
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

fn wrap(
    font: &Font,
    glyphs: Option<&GlyphCache>,
    text: &str,
    size: f32,
    max_width: f32,
) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_inclusive(' ') {
            let candidate = format!("{line}{word}");
            if !line.is_empty() && line_width(font, glyphs, &candidate, size) > max_width {
                lines.push(std::mem::take(&mut line));
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn line_width(font: &Font, glyphs: Option<&GlyphCache>, text: &str, size: f32) -> f32 {
    text.chars()
        .map(|ch| rasterize(font, glyphs, ch, size).0.advance_width)
        .sum()
}

fn rasterize(
    font: &Font,
    glyphs: Option<&GlyphCache>,
    ch: char,
    size: f32,
) -> (Metrics, Vec<u8>) {
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
                blend(image, x, y, coverage);
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

fn blend(image: &mut RgbaImage, x: i32, y: i32, coverage: u8) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as u32;
    let y = y as u32;
    if x >= image.width() || y >= image.height() {
        return;
    }
    let pixel = image.get_pixel(x, y);
    let keep = 255 - coverage as u16;
    let mix = |channel: u8| ((u16::from(channel) * keep) / 255) as u8;
    image.put_pixel(
        x,
        y,
        Rgba([mix(pixel[0]), mix(pixel[1]), mix(pixel[2]), 255]),
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
