use std::fs;
use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Object, Stream};

use crate::struct_to_pdf::barcode::{self, Symbol};
use crate::struct_to_pdf::geometry::{frame_matrix, media_size, pdf_point, to_points};
use crate::struct_to_pdf::text::{self, PendingFont};
use crate::struct_to_pdf::{Barcode, ContentEntry, Page, Photo, RenderError, Units};

pub struct PaintedPage {
    pub content: Vec<u8>,
    pub xobjects: Vec<(String, Stream)>,
    pub fonts: Vec<PendingFont>,
    pub ext_gstates: Vec<(String, f32)>,
    pub forms: Vec<FormPaint>,
}

/// One entry painted at full opacity, then faded once when the page draws it.
pub struct FormPaint {
    pub name: String,
    pub content: Vec<u8>,
    pub bbox: [f32; 4],
    pub fonts: Vec<PendingFont>,
    pub xobjects: Vec<(String, Stream)>,
}

pub fn paint_contents(page: &Page, base_dir: &Path) -> Result<PaintedPage, RenderError> {
    let mut operations = Vec::new();
    let mut xobjects = Vec::new();
    let mut fonts = Vec::new();
    let mut ext_gstates = Vec::new();
    let mut forms = Vec::new();
    let mut font_index = 0;
    let mut gs_index = 0;
    let (media_width, media_height) = media_size(page);

    if let Some(background) = page.page_size.background {
        let (red, green, blue) = background.components();
        operations.push(Operation::new(
            "rg",
            vec![Object::Real(red), Object::Real(green), Object::Real(blue)],
        ));
        operations.push(Operation::new(
            "re",
            vec![
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(media_width),
                Object::Real(media_height),
            ],
        ));
        operations.push(Operation::new("f", vec![]));
    }

    if page
        .contents
        .iter()
        .any(|entry| !matches!(entry, ContentEntry::Text(_)))
    {
        operations.push(Operation::new("q", vec![]));
        operations.push(Operation::new(
            "re",
            vec![
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(media_width),
                Object::Real(media_height),
            ],
        ));
        operations.push(Operation::new("W", vec![]));
        operations.push(Operation::new("n", vec![]));
    }

    for entry in &page.contents {
        let frame = frame_paint(page, entry, &mut gs_index);
        let mut entry_ops = Vec::new();
        let mut entry_fonts = Vec::new();
        let mut entry_xobjects = Vec::new();
        match entry {
            ContentEntry::Text(text) => {
                let drawn = text::draw_text(page, text, base_dir, &mut font_index)?;
                entry_ops.extend(drawn.operations);
                entry_fonts.extend(drawn.fonts);
            }
            ContentEntry::Barcode(code) => {
                draw_barcode(
                    page,
                    code,
                    &mut entry_ops,
                    &mut entry_xobjects,
                    xobjects.len(),
                )?;
            }
            ContentEntry::Photo(photo) => {
                draw_photo(
                    page,
                    photo,
                    &mut entry_ops,
                    &mut entry_xobjects,
                    xobjects.len(),
                )?;
            }
            ContentEntry::Image(placement) => {
                let resource =
                    page.images
                        .get(&placement.resource_name)
                        .ok_or_else(|| RenderError {
                            context: placement.id.clone(),
                            message: format!("unknown image resource {}", placement.resource_name),
                        })?;
                let format = resource
                    .file_format
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !matches!(format.as_str(), "jpg" | "jpeg" | "bmp") {
                    return Err(RenderError {
                        context: placement.id.clone(),
                        message: format!("{format} not in v0 yet"),
                    });
                }
                let bytes = if let Some(bytes) = resource.bytes.clone() {
                    bytes
                } else {
                    let relative =
                        resource
                            .extracted_path
                            .as_deref()
                            .ok_or_else(|| RenderError {
                                context: placement.id.clone(),
                                message: "extracted_path is missing".to_string(),
                            })?;
                    let path = base_dir.join(relative);
                    fs::read(&path).map_err(|_| RenderError {
                        context: placement.id.clone(),
                        message: format!("image file not found: {}", path.display()),
                    })?
                };

                let name = image_name(xobjects.len(), entry_xobjects.len());
                let stream = match format.as_str() {
                    "jpg" | "jpeg" => {
                        let pixels_wide = resource.width.ok_or_else(|| RenderError {
                            context: placement.id.clone(),
                            message: "image width is missing".to_string(),
                        })?;
                        let pixels_high = resource.height.ok_or_else(|| RenderError {
                            context: placement.id.clone(),
                            message: "image height is missing".to_string(),
                        })?;
                        Stream::new(
                            dictionary! {
                                "Type" => "XObject",
                                "Subtype" => "Image",
                                "Width" => pixels_wide as i64,
                                "Height" => pixels_high as i64,
                                "ColorSpace" => "DeviceRGB",
                                "BitsPerComponent" => 8,
                                "Filter" => "DCTDecode",
                            },
                            bytes,
                        )
                        .with_compression(false)
                    }
                    "bmp" => {
                        let decoded = decode_bmp(&bytes, &placement.id)?;
                        let (width, height, color_space, samples) = match decoded {
                            BmpSamples::Rgb {
                                width,
                                height,
                                samples,
                            } => (width, height, "DeviceRGB", samples),
                            BmpSamples::Gray {
                                width,
                                height,
                                samples,
                            } => (width, height, "DeviceGray", samples),
                        };
                        Stream::new(
                            dictionary! {
                                "Type" => "XObject",
                                "Subtype" => "Image",
                                "Width" => width as i64,
                                "Height" => height as i64,
                                "ColorSpace" => color_space,
                                "BitsPerComponent" => 8,
                            },
                            samples,
                        )
                        .with_compression(false)
                    }
                    _ => unreachable!("format was checked"),
                };

                let units = page.page_size.units;
                let width = to_points(placement.width, units);
                let height = to_points(placement.height, units);
                let (pdf_x, pdf_y) =
                    pdf_point(page, placement.pos_x, placement.pos_y + placement.height);
                entry_ops.push(Operation::new("q", vec![]));
                entry_ops.push(Operation::new(
                    "cm",
                    vec![
                        Object::Real(width),
                        Object::Real(0.0),
                        Object::Real(0.0),
                        Object::Real(height),
                        Object::Real(pdf_x),
                        Object::Real(pdf_y),
                    ],
                ));
                entry_ops.push(Operation::new(
                    "Do",
                    vec![Object::Name(name.as_bytes().to_vec())],
                ));
                entry_ops.push(Operation::new("Q", vec![]));
                entry_xobjects.push((name, stream));
            }
        }
        place_entry(
            entry,
            frame,
            entry_ops,
            entry_fonts,
            entry_xobjects,
            &mut operations,
            &mut xobjects,
            &mut fonts,
            &mut ext_gstates,
            &mut forms,
        )?;
    }

    if page
        .contents
        .iter()
        .any(|entry| !matches!(entry, ContentEntry::Text(_)))
    {
        operations.push(Operation::new("Q", vec![]));
    }

    let content = Content { operations }.encode().map_err(|err| RenderError {
        context: "page".to_string(),
        message: err.to_string(),
    })?;
    Ok(PaintedPage {
        content,
        xobjects,
        fonts,
        ext_gstates,
        forms,
    })
}

struct FramePaint {
    transform: Option<Operation>,
    graphics: Option<(String, f32)>,
    prefix: Vec<Operation>,
    suffix: Vec<Operation>,
    bbox: [f32; 4],
}

fn stacks_with_frame(entry: &ContentEntry) -> bool {
    let frame = entry.frame();
    frame.background.is_some() || frame.border.is_some()
}

fn place_entry(
    entry: &ContentEntry,
    frame: FramePaint,
    entry_ops: Vec<Operation>,
    entry_fonts: Vec<PendingFont>,
    entry_xobjects: Vec<(String, Stream)>,
    operations: &mut Vec<Operation>,
    xobjects: &mut Vec<(String, Stream)>,
    fonts: &mut Vec<PendingFont>,
    ext_gstates: &mut Vec<(String, f32)>,
    forms: &mut Vec<FormPaint>,
) -> Result<(), RenderError> {
    if entry.frame().is_plain() {
        operations.extend(entry_ops);
        fonts.extend(entry_fonts);
        xobjects.extend(entry_xobjects);
        return Ok(());
    }
    operations.push(Operation::new("q", vec![]));
    if let Some(transform) = frame.transform {
        operations.push(transform);
    }
    let grouped = frame.graphics.is_some() && stacks_with_frame(entry);
    if grouped {
        let mut form_ops = frame.prefix;
        form_ops.extend(entry_ops);
        form_ops.extend(frame.suffix);
        let content = Content {
            operations: form_ops,
        }
        .encode()
        .map_err(|err| RenderError {
            context: entry_id(entry),
            message: err.to_string(),
        })?;
        let name = format!("Fm{}", forms.len() + 1);
        let (gs_name, alpha) = frame.graphics.expect("a grouped entry has an opacity");
        operations.push(Operation::new(
            "gs",
            vec![Object::Name(gs_name.as_bytes().to_vec())],
        ));
        ext_gstates.push((gs_name, alpha));
        operations.push(Operation::new(
            "Do",
            vec![Object::Name(name.as_bytes().to_vec())],
        ));
        forms.push(FormPaint {
            name,
            content,
            bbox: frame.bbox,
            fonts: entry_fonts,
            xobjects: entry_xobjects,
        });
    } else {
        if let Some((gs_name, alpha)) = frame.graphics {
            operations.push(Operation::new(
                "gs",
                vec![Object::Name(gs_name.as_bytes().to_vec())],
            ));
            ext_gstates.push((gs_name, alpha));
        }
        operations.extend(frame.prefix);
        operations.extend(entry_ops);
        operations.extend(frame.suffix);
        fonts.extend(entry_fonts);
        xobjects.extend(entry_xobjects);
    }
    operations.push(Operation::new("Q", vec![]));
    Ok(())
}

fn entry_id(entry: &ContentEntry) -> String {
    match entry {
        ContentEntry::Text(text) => text.id.clone(),
        ContentEntry::Image(image) => image.id.clone(),
        ContentEntry::Barcode(code) => code.id.clone(),
        ContentEntry::Photo(photo) => photo.id.clone(),
    }
}

fn frame_paint(page: &Page, entry: &ContentEntry, gs_index: &mut usize) -> FramePaint {
    let frame = entry.frame();
    if frame.is_plain() {
        return FramePaint {
            transform: None,
            graphics: None,
            prefix: Vec::new(),
            suffix: Vec::new(),
            bbox: [0.0, 0.0, 0.0, 0.0],
        };
    }
    let (pos_x, pos_y, width, height) = entry.bounds();
    let units = page.page_size.units;
    let (left, bottom) = pdf_point(page, pos_x, pos_y + height);
    let box_w = to_points(width, units);
    let box_h = to_points(height, units);
    let radius = to_points(frame.border_radius, units);
    let mut prefix = Vec::new();
    let (anchor_x, anchor_y) = pdf_point(page, pos_x, pos_y);
    let transformed =
        frame.scale_x != 1.0 || frame.scale_y != 1.0 || frame.skew_x != 0.0 || frame.skew_y != 0.0;
    let transform = transformed.then(|| {
        let matrix = frame_matrix(
            f64::from(anchor_x),
            f64::from(anchor_y),
            frame.scale_x,
            frame.scale_y,
            frame.skew_x,
            frame.skew_y,
        );
        Operation::new(
            "cm",
            matrix
                .into_iter()
                .map(|value| Object::Real(value as f32))
                .collect(),
        )
    });
    let graphics = (frame.opacity != 1.0).then(|| {
        *gs_index += 1;
        (format!("Gs{gs_index}"), frame.opacity as f32)
    });
    if let Some(color) = frame.background {
        let (red, green, blue) = color.components();
        prefix.push(Operation::new(
            "rg",
            vec![Object::Real(red), Object::Real(green), Object::Real(blue)],
        ));
        append_box_path(&mut prefix, left, bottom, box_w, box_h, radius);
        prefix.push(Operation::new("f", vec![]));
    }
    if let Some(border) = &frame.border {
        let (red, green, blue) = border.color.components();
        prefix.push(Operation::new(
            "RG",
            vec![Object::Real(red), Object::Real(green), Object::Real(blue)],
        ));
        prefix.push(Operation::new(
            "w",
            vec![Object::Real(to_points(border.width, units))],
        ));
        append_box_path(&mut prefix, left, bottom, box_w, box_h, radius);
        prefix.push(Operation::new("S", vec![]));
    }
    let mut suffix = Vec::new();
    if frame.border_radius > 0.0 {
        prefix.push(Operation::new("q", vec![]));
        append_box_path(&mut prefix, left, bottom, box_w, box_h, radius);
        prefix.push(Operation::new("W", vec![]));
        prefix.push(Operation::new("n", vec![]));
        suffix.push(Operation::new("Q", vec![]));
    }
    let pad = frame
        .border
        .as_ref()
        .map(|border| to_points(border.width, units))
        .unwrap_or(0.0)
        + match entry {
            ContentEntry::Text(text) => to_points(text.text.font_size, units),
            _ => 0.0,
        };
    FramePaint {
        transform,
        graphics,
        prefix,
        suffix,
        bbox: [
            left - pad,
            bottom - pad,
            left + box_w + pad,
            bottom + box_h + pad,
        ],
    }
}

fn append_box_path(ops: &mut Vec<Operation>, x: f32, y: f32, w: f32, h: f32, radius: f32) {
    let radius = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if radius <= 0.0 {
        ops.push(Operation::new(
            "re",
            vec![
                Object::Real(x),
                Object::Real(y),
                Object::Real(w),
                Object::Real(h),
            ],
        ));
        return;
    }
    let k = 0.552_284_75 * radius;
    let real = Object::Real;
    ops.push(Operation::new("m", vec![real(x + radius), real(y)]));
    ops.push(Operation::new("l", vec![real(x + w - radius), real(y)]));
    ops.push(Operation::new(
        "c",
        vec![
            real(x + w - radius + k),
            real(y),
            real(x + w),
            real(y + radius - k),
            real(x + w),
            real(y + radius),
        ],
    ));
    ops.push(Operation::new("l", vec![real(x + w), real(y + h - radius)]));
    ops.push(Operation::new(
        "c",
        vec![
            real(x + w),
            real(y + h - radius + k),
            real(x + w - radius + k),
            real(y + h),
            real(x + w - radius),
            real(y + h),
        ],
    ));
    ops.push(Operation::new("l", vec![real(x + radius), real(y + h)]));
    ops.push(Operation::new(
        "c",
        vec![
            real(x + radius - k),
            real(y + h),
            real(x),
            real(y + h - radius + k),
            real(x),
            real(y + h - radius),
        ],
    ));
    ops.push(Operation::new("l", vec![real(x), real(y + radius)]));
    ops.push(Operation::new(
        "c",
        vec![
            real(x),
            real(y + radius - k),
            real(x + radius - k),
            real(y),
            real(x + radius),
            real(y),
        ],
    ));
    ops.push(Operation::new("h", vec![]));
}

fn image_name(committed: usize, pending: usize) -> String {
    format!("Im{}", committed + pending + 1)
}

fn draw_barcode(
    page: &Page,
    code: &Barcode,
    operations: &mut Vec<Operation>,
    xobjects: &mut Vec<(String, Stream)>,
    committed: usize,
) -> Result<(), RenderError> {
    let payload = code
        .content
        .clone()
        .or_else(|| code.preentered.clone())
        .unwrap_or_default();
    let (width, height, samples) = match barcode::symbol(&code.barcode_type, &payload, &code.id)? {
        Symbol::Bars(modules) => {
            let width = modules.len() as u32;
            let samples = modules
                .into_iter()
                .map(|on| if on { 0 } else { 255 })
                .collect();
            (width, 1, samples)
        }
        Symbol::Qr { width, modules } => {
            let (span, samples, pos_x, pos_y, side) = qr_bitmap(page, code, width, &modules);
            place_raster(
                page,
                operations,
                xobjects,
                committed,
                pos_x,
                pos_y,
                side,
                side,
                span,
                span,
                "DeviceGray",
                samples,
                true,
            );
            return Ok(());
        }
    };
    place_raster(
        page,
        operations,
        xobjects,
        committed,
        code.pos_x,
        code.pos_y,
        code.width,
        code.height,
        width,
        height,
        "DeviceGray",
        samples,
        true,
    );
    Ok(())
}

const QR_DPI: f64 = 400.0;

fn qr_bitmap(
    page: &Page,
    code: &Barcode,
    modules_wide: usize,
    modules: &[bool],
) -> (u32, Vec<u8>, f64, f64, f64) {
    let units = page.page_size.units;
    let box_px = qr_pixels(code.width, units).min(qr_pixels(code.height, units));
    let module = (box_px / modules_wide as u32).max(1);
    let span = module * modules_wide as u32;
    let mut samples = vec![255u8; (span as usize) * (span as usize)];
    for y in 0..modules_wide {
        for x in 0..modules_wide {
            if !modules[y * modules_wide + x] {
                continue;
            }
            for py in 0..module {
                for px in 0..module {
                    let row = y as u32 * module + py;
                    let col = x as u32 * module + px;
                    samples[(row * span + col) as usize] = 0;
                }
            }
        }
    }
    let side = qr_length(span, units);
    let pos_x = code.pos_x + (code.width - side) / 2.0;
    let pos_y = code.pos_y + (code.height - side) / 2.0;
    (span, samples, pos_x, pos_y, side)
}

fn qr_pixels(value: f64, units: Units) -> u32 {
    let inches = match units {
        Units::Points => value / 72.0,
        Units::Mm => value / 25.4,
    };
    (inches * QR_DPI).round().max(1.0) as u32
}

fn qr_length(pixels: u32, units: Units) -> f64 {
    let inches = pixels as f64 / QR_DPI;
    match units {
        Units::Points => inches * 72.0,
        Units::Mm => inches * 25.4,
    }
}

fn draw_photo(
    page: &Page,
    photo: &Photo,
    operations: &mut Vec<Operation>,
    xobjects: &mut Vec<(String, Stream)>,
    committed: usize,
) -> Result<(), RenderError> {
    let Some(bytes) = photo.bytes.as_deref() else {
        return Ok(());
    };
    if bytes.is_empty() {
        return Ok(());
    }
    let (width, height, samples) = raster_image(bytes, &photo.id)?;
    place_raster(
        page,
        operations,
        xobjects,
        committed,
        photo.pos_x,
        photo.pos_y,
        photo.width,
        photo.height,
        width,
        height,
        "DeviceRGB",
        samples,
        false,
    );
    Ok(())
}

fn raster_image(bytes: &[u8], id: &str) -> Result<(u32, u32, Vec<u8>), RenderError> {
    if bytes.starts_with(b"BM") {
        return match decode_bmp(bytes, id)? {
            BmpSamples::Rgb {
                width,
                height,
                samples,
            } => Ok((width, height, samples)),
            BmpSamples::Gray {
                width,
                height,
                samples,
            } => {
                let rgb = samples
                    .into_iter()
                    .flat_map(|sample| [sample, sample, sample]);
                Ok((width, height, rgb.collect()))
            }
        };
    }
    if bytes.starts_with(b"\x89PNG") || bytes.starts_with(&[0xFF, 0xD8]) {
        let image = image::load_from_memory(bytes).map_err(|err| RenderError {
            context: id.to_string(),
            message: err.to_string(),
        })?;
        let rgb = image.to_rgb8();
        return Ok((rgb.width(), rgb.height(), rgb.into_raw()));
    }
    Err(RenderError {
        context: id.to_string(),
        message: "photo must be jpeg, bmp, or png".to_string(),
    })
}

fn place_raster(
    page: &Page,
    operations: &mut Vec<Operation>,
    xobjects: &mut Vec<(String, Stream)>,
    committed: usize,
    pos_x: f64,
    pos_y: f64,
    box_width: f64,
    box_height: f64,
    pixels_wide: u32,
    pixels_high: u32,
    color_space: &str,
    samples: Vec<u8>,
    sharp: bool,
) {
    let name = image_name(committed, xobjects.len());
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => pixels_wide as i64,
            "Height" => pixels_high as i64,
            "ColorSpace" => color_space,
            "BitsPerComponent" => 8,
        },
        samples,
    )
    .with_compression(false);
    if sharp {
        stream.dict.set("Interpolate", Object::Boolean(false));
    }
    let units = page.page_size.units;
    let width = to_points(box_width, units);
    let height = to_points(box_height, units);
    let (pdf_x, pdf_y) = pdf_point(page, pos_x, pos_y + box_height);
    operations.push(Operation::new("q", vec![]));
    operations.push(Operation::new(
        "cm",
        vec![
            Object::Real(width),
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(height),
            Object::Real(pdf_x),
            Object::Real(pdf_y),
        ],
    ));
    operations.push(Operation::new(
        "Do",
        vec![Object::Name(name.as_bytes().to_vec())],
    ));
    operations.push(Operation::new("Q", vec![]));
    xobjects.push((name, stream));
}

enum BmpSamples {
    Rgb {
        width: u32,
        height: u32,
        samples: Vec<u8>,
    },
    Gray {
        width: u32,
        height: u32,
        samples: Vec<u8>,
    },
}

fn decode_bmp(bytes: &[u8], id: &str) -> Result<BmpSamples, RenderError> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return Err(bmp_error(id, "not a bmp"));
    }
    let pixel_offset = u32_at(bytes, 10) as usize;
    let header_size = u32_at(bytes, 14);
    if header_size != 40 {
        return Err(bmp_error(id, "unsupported bmp header"));
    }
    let width = i32_at(bytes, 18);
    let height_raw = i32_at(bytes, 22);
    let planes = u16_at(bytes, 26);
    let bit_count = u16_at(bytes, 28);
    let compression = u32_at(bytes, 30);
    let colors_used = u32_at(bytes, 46);
    if planes != 1 || compression != 0 || width <= 0 || height_raw == 0 {
        return Err(bmp_error(id, "unsupported bmp header"));
    }
    let top_down = height_raw < 0;
    let height = height_raw.unsigned_abs();
    let width = width as u32;
    if !matches!(bit_count, 8 | 24 | 32) {
        return Err(bmp_error(
            id,
            &format!("bit depth {bit_count} is not supported"),
        ));
    }

    let width_px = usize::try_from(width).map_err(|_| bmp_error(id, "unsupported bmp header"))?;
    let stride = ((width_px * bit_count as usize + 31) / 32) * 4;
    let pixel_bytes = stride
        .checked_mul(height as usize)
        .ok_or_else(|| bmp_error(id, "bmp is truncated"))?;
    let pixel_end = pixel_offset
        .checked_add(pixel_bytes)
        .ok_or_else(|| bmp_error(id, "bmp is truncated"))?;
    if pixel_end > bytes.len() {
        return Err(bmp_error(id, "bmp is truncated"));
    }

    let mut rows = Vec::with_capacity(height as usize);
    for file_row in 0..height as usize {
        let start = pixel_offset + file_row * stride;
        rows.push(&bytes[start..start + stride]);
    }
    if !top_down {
        rows.reverse();
    }

    if bit_count == 8 {
        let colors = if colors_used == 0 {
            256
        } else {
            colors_used as usize
        };
        let palette_at = 54;
        let palette_end = palette_at + colors * 4;
        if palette_end > bytes.len() || palette_end > pixel_offset {
            return Err(bmp_error(id, "bmp is truncated"));
        }
        let mut samples = Vec::with_capacity(width as usize * height as usize);
        for row in rows {
            for x in 0..width as usize {
                let index = row[x] as usize;
                if index >= colors {
                    return Err(bmp_error(id, "bmp palette index is out of range"));
                }
                let entry = &bytes[palette_at + index * 4..palette_at + index * 4 + 3];
                if entry[0] != entry[1] || entry[1] != entry[2] {
                    return Err(bmp_error(id, "bmp palette is not gray"));
                }
                samples.push(entry[2]);
            }
        }
        return Ok(BmpSamples::Gray {
            width,
            height,
            samples,
        });
    }

    let channels = if bit_count == 32 { 4 } else { 3 };
    let mut samples = Vec::with_capacity(width as usize * height as usize * 3);
    for row in rows {
        for x in 0..width as usize {
            let pixel = &row[x * channels..x * channels + 3];
            samples.push(pixel[2]);
            samples.push(pixel[1]);
            samples.push(pixel[0]);
        }
    }
    Ok(BmpSamples::Rgb {
        width,
        height,
        samples,
    })
}

fn bmp_error(id: &str, message: &str) -> RenderError {
    RenderError {
        context: id.to_string(),
        message: message.to_string(),
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
