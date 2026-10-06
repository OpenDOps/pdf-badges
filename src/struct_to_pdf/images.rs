use std::fs;
use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Object, Stream};

use crate::struct_to_pdf::barcode::{self, Symbol};
use crate::struct_to_pdf::geometry::{media_size, pdf_point, to_points};
use crate::struct_to_pdf::text::{self, PendingFont};
use crate::struct_to_pdf::{Barcode, ContentEntry, Page, Photo, RenderError};

pub struct PaintedPage {
    pub content: Vec<u8>,
    pub xobjects: Vec<(String, Stream)>,
    pub fonts: Vec<PendingFont>,
}

pub fn paint_contents(page: &Page, base_dir: &Path) -> Result<PaintedPage, RenderError> {
    let mut operations = Vec::new();
    let mut xobjects = Vec::new();
    let mut fonts = Vec::new();
    let mut font_index = 0;

    if page
        .contents
        .iter()
        .any(|entry| !matches!(entry, ContentEntry::Text(_)))
    {
        let (media_width, media_height) = media_size(page);
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
        match entry {
            ContentEntry::Text(text) => {
                let drawn = text::draw_text(page, text, base_dir, &mut font_index)?;
                operations.extend(drawn.operations);
                fonts.extend(drawn.fonts);
            }
            ContentEntry::Barcode(code) => {
                draw_barcode(page, code, &mut operations, &mut xobjects)?;
            }
            ContentEntry::Photo(photo) => {
                draw_photo(page, photo, &mut operations, &mut xobjects)?;
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
                let relative = resource
                    .extracted_path
                    .as_deref()
                    .ok_or_else(|| RenderError {
                        context: placement.id.clone(),
                        message: "extracted_path is missing".to_string(),
                    })?;
                let path = base_dir.join(relative);
                let bytes = fs::read(&path).map_err(|_| RenderError {
                    context: placement.id.clone(),
                    message: format!("image file not found: {}", path.display()),
                })?;

                let name = format!("Im{}", xobjects.len() + 1);
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
        }
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
    })
}

fn draw_barcode(
    page: &Page,
    code: &Barcode,
    operations: &mut Vec<Operation>,
    xobjects: &mut Vec<(String, Stream)>,
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
            let module = 4u32;
            let span = width as u32 * module;
            let mut samples = vec![255u8; (span * span) as usize];
            for y in 0..width {
                for x in 0..width {
                    if !modules[y * width + x] {
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
            (span, span, samples)
        }
    };
    place_raster(
        page,
        operations,
        xobjects,
        code.pos_x,
        code.pos_y,
        code.width,
        code.height,
        width,
        height,
        "DeviceGray",
        samples,
    );
    Ok(())
}

fn draw_photo(
    page: &Page,
    photo: &Photo,
    operations: &mut Vec<Operation>,
    xobjects: &mut Vec<(String, Stream)>,
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
        photo.pos_x,
        photo.pos_y,
        photo.width,
        photo.height,
        width,
        height,
        "DeviceRGB",
        samples,
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
    pos_x: f64,
    pos_y: f64,
    box_width: f64,
    box_height: f64,
    pixels_wide: u32,
    pixels_high: u32,
    color_space: &str,
    samples: Vec<u8>,
) {
    let name = format!("Im{}", xobjects.len() + 1);
    let stream = Stream::new(
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
