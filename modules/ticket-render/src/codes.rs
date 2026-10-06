use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use qrcode::{Color, EcLevel, QrCode};

use crate::page::{err, PageError};

const EAN_BARS: [[u8; 4]; 10] = [
    [3, 2, 1, 1],
    [2, 2, 2, 1],
    [2, 1, 2, 2],
    [1, 4, 1, 1],
    [1, 1, 3, 2],
    [1, 2, 3, 1],
    [1, 1, 1, 4],
    [1, 3, 1, 2],
    [1, 2, 1, 3],
    [3, 1, 1, 2],
];

const EAN_PARITY: [[u8; 6]; 10] = [
    [0, 0, 0, 0, 0, 0],
    [0, 0, 1, 0, 1, 1],
    [0, 0, 1, 1, 0, 1],
    [0, 0, 1, 1, 1, 0],
    [0, 1, 0, 0, 1, 1],
    [0, 1, 1, 0, 0, 1],
    [0, 1, 1, 1, 0, 0],
    [0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 0],
    [0, 1, 1, 0, 1, 0],
];

const CODE128: [[u8; 6]; 106] = [
    [2, 1, 2, 2, 2, 2],
    [2, 2, 2, 1, 2, 2],
    [2, 2, 2, 2, 2, 1],
    [1, 2, 1, 2, 2, 3],
    [1, 2, 1, 3, 2, 2],
    [1, 3, 1, 2, 2, 2],
    [1, 2, 2, 2, 1, 3],
    [1, 2, 2, 3, 1, 2],
    [1, 3, 2, 2, 1, 2],
    [2, 2, 1, 2, 1, 3],
    [2, 2, 1, 3, 1, 2],
    [2, 3, 1, 2, 1, 2],
    [1, 1, 2, 2, 3, 2],
    [1, 2, 2, 1, 3, 2],
    [1, 2, 2, 2, 3, 1],
    [1, 1, 3, 2, 2, 2],
    [1, 2, 3, 1, 2, 2],
    [1, 2, 3, 2, 2, 1],
    [2, 2, 3, 2, 1, 1],
    [2, 2, 1, 1, 3, 2],
    [2, 2, 1, 2, 3, 1],
    [2, 1, 3, 2, 1, 2],
    [2, 2, 3, 1, 1, 2],
    [3, 1, 2, 1, 3, 1],
    [3, 1, 1, 2, 2, 2],
    [3, 2, 1, 1, 2, 2],
    [3, 2, 1, 2, 2, 1],
    [3, 1, 2, 2, 1, 2],
    [3, 2, 2, 1, 1, 2],
    [3, 2, 2, 2, 1, 1],
    [2, 1, 2, 1, 2, 3],
    [2, 1, 2, 3, 2, 1],
    [2, 3, 2, 1, 2, 1],
    [1, 1, 1, 3, 2, 3],
    [1, 3, 1, 1, 2, 3],
    [1, 3, 1, 3, 2, 1],
    [1, 1, 2, 3, 1, 3],
    [1, 3, 2, 1, 1, 3],
    [1, 3, 2, 3, 1, 1],
    [2, 1, 1, 3, 1, 3],
    [2, 3, 1, 1, 1, 3],
    [2, 3, 1, 3, 1, 1],
    [1, 1, 2, 1, 3, 3],
    [1, 1, 2, 3, 3, 1],
    [1, 3, 2, 1, 3, 1],
    [1, 1, 3, 1, 2, 3],
    [1, 1, 3, 3, 2, 1],
    [1, 3, 3, 1, 2, 1],
    [3, 1, 3, 1, 2, 1],
    [2, 1, 1, 3, 3, 1],
    [2, 3, 1, 1, 3, 1],
    [2, 1, 3, 1, 1, 3],
    [2, 1, 3, 3, 1, 1],
    [2, 1, 3, 1, 3, 1],
    [3, 1, 1, 1, 2, 3],
    [3, 1, 1, 3, 2, 1],
    [3, 3, 1, 1, 2, 1],
    [3, 1, 2, 1, 1, 3],
    [3, 1, 2, 3, 1, 1],
    [3, 3, 2, 1, 1, 1],
    [3, 1, 4, 1, 1, 1],
    [2, 2, 1, 4, 1, 1],
    [4, 3, 1, 1, 1, 1],
    [1, 1, 1, 2, 2, 4],
    [1, 1, 1, 4, 2, 2],
    [1, 2, 1, 1, 2, 4],
    [1, 2, 1, 4, 2, 1],
    [1, 4, 1, 1, 2, 2],
    [1, 4, 1, 2, 2, 1],
    [1, 1, 2, 2, 1, 4],
    [1, 1, 2, 4, 1, 2],
    [1, 2, 2, 1, 1, 4],
    [1, 2, 2, 4, 1, 1],
    [1, 4, 2, 1, 1, 2],
    [1, 4, 2, 2, 1, 1],
    [2, 4, 1, 2, 1, 1],
    [2, 2, 1, 1, 1, 4],
    [4, 1, 3, 1, 1, 1],
    [2, 4, 1, 1, 1, 2],
    [1, 3, 4, 1, 1, 1],
    [1, 1, 1, 2, 4, 2],
    [1, 2, 1, 1, 4, 2],
    [1, 2, 1, 2, 4, 1],
    [1, 1, 4, 2, 1, 2],
    [1, 2, 4, 1, 1, 2],
    [1, 2, 4, 2, 1, 1],
    [4, 1, 1, 2, 1, 2],
    [4, 2, 1, 1, 1, 2],
    [4, 2, 1, 2, 1, 1],
    [2, 1, 2, 1, 4, 1],
    [2, 1, 4, 1, 2, 1],
    [4, 1, 2, 1, 2, 1],
    [1, 1, 1, 1, 4, 3],
    [1, 1, 1, 3, 4, 1],
    [1, 3, 1, 1, 4, 1],
    [1, 1, 4, 1, 1, 3],
    [1, 1, 4, 3, 1, 1],
    [4, 1, 1, 1, 1, 3],
    [4, 1, 1, 3, 1, 1],
    [1, 1, 3, 1, 4, 1],
    [1, 1, 4, 1, 3, 1],
    [3, 1, 1, 1, 4, 1],
    [4, 1, 1, 1, 3, 1],
    [2, 1, 1, 4, 1, 2],
    [2, 1, 1, 2, 1, 4],
    [2, 1, 1, 2, 3, 2],
];

const CODE128_START_B: usize = 104;
const CODE128_STOP: [u8; 7] = [2, 3, 3, 1, 1, 1, 2];

pub(crate) enum Symbol {
    Bars(Vec<bool>),
    Qr { width: usize, modules: Vec<bool> },
}

pub(crate) fn symbol(barcode_type: &str, text: &str) -> Result<Symbol, PageError> {
    match barcode_type.to_ascii_lowercase().as_str() {
        "ean13" | "ean-13" => Ok(Symbol::Bars(ean13_modules(text)?)),
        "code128" | "code-128" => Ok(Symbol::Bars(code128_modules(text)?)),
        "qr" => {
            let (width, modules) = qr_modules(text)?;
            Ok(Symbol::Qr { width, modules })
        }
        _ => Err(err(format!("unknown barcode type {barcode_type}"))),
    }
}

pub(crate) fn ean13_modules(code: &str) -> Result<Vec<bool>, PageError> {
    if code.len() != 13 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(err(format!("ean13 {code}")));
    }
    let digits: Vec<usize> = code.bytes().map(|byte| (byte - b'0') as usize).collect();
    let mut bits = vec![false; 9];
    let mut black = true;
    push(&mut bits, &[1, 1, 1], &mut black);
    for index in 0..6 {
        let stripes = EAN_BARS[digits[index + 1]];
        if EAN_PARITY[digits[0]][index] == 0 {
            push(&mut bits, &stripes, &mut black);
        } else {
            push(
                &mut bits,
                &[stripes[3], stripes[2], stripes[1], stripes[0]],
                &mut black,
            );
        }
    }
    push(&mut bits, &[1, 1, 1, 1, 1], &mut black);
    for index in 7..13 {
        push(&mut bits, &EAN_BARS[digits[index]], &mut black);
    }
    push(&mut bits, &[1, 1, 1], &mut black);
    bits.extend(std::iter::repeat(false).take(9));
    Ok(bits)
}

pub(crate) fn code128_modules(code: &str) -> Result<Vec<bool>, PageError> {
    let mut values = Vec::with_capacity(code.len());
    for ch in code.chars() {
        let byte = ch as u32;
        if !(32..127).contains(&byte) {
            return Err(err(format!("code 128 {code}")));
        }
        values.push((byte - 32) as usize);
    }
    if values.is_empty() {
        return Err(err("code 128"));
    }
    let mut sum = CODE128_START_B;
    for (index, value) in values.iter().enumerate() {
        sum += (index + 1) * value;
    }
    let check = sum % 103;
    let mut bits = Vec::new();
    let mut black = true;
    push(&mut bits, &CODE128[CODE128_START_B], &mut black);
    for value in values {
        push(&mut bits, &CODE128[value], &mut black);
    }
    push(&mut bits, &CODE128[check], &mut black);
    push(&mut bits, &CODE128_STOP, &mut black);
    Ok(bits)
}

pub fn render_qr(text: &str, size_px: u32) -> Result<Vec<u8>, PageError> {
    if size_px == 0 {
        return Err(err("qr size"));
    }
    let (width, modules) = qr_modules(text)?;
    let module = size_px / width as u32;
    if module == 0 {
        return Err(err("qr size"));
    }
    let span = module * width as u32;
    let origin = (size_px - span) / 2;
    let mut image = RgbaImage::from_pixel(size_px, size_px, Rgba([255, 255, 255, 255]));
    for y in 0..width {
        for x in 0..width {
            if !modules[y * width + x] {
                continue;
            }
            let left = origin + x as u32 * module;
            let top = origin + y as u32 * module;
            for py in top..top + module {
                for px in left..left + module {
                    image.put_pixel(px, py, Rgba([0, 0, 0, 255]));
                }
            }
        }
    }
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| err(error.to_string()))?;
    Ok(bytes)
}

pub(crate) fn qr_modules(text: &str) -> Result<(usize, Vec<bool>), PageError> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::H)
        .map_err(|error| err(error.to_string()))?;
    let width = code.width();
    let mut modules = Vec::with_capacity(width * width);
    for y in 0..width {
        for x in 0..width {
            modules.push(code[(x, y)] == Color::Dark);
        }
    }
    Ok((width, modules))
}

fn push(bits: &mut Vec<bool>, widths: &[u8], black: &mut bool) {
    for &width in widths {
        for _ in 0..width {
            bits.push(*black);
        }
        *black = !*black;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Alignment, Area, AreaKind, Layout, Rect};
    use crate::page::area_px;
    use crate::render_png;
    use std::collections::HashMap;

    fn bits(modules: &[bool]) -> String {
        modules
            .iter()
            .map(|on| if *on { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn ean13_modules() {
        let modules = super::ean13_modules("9891160081678").unwrap();
        let text = bits(&modules);
        assert_eq!(
            text,
            "00000000010101101110010111011001100110010000101000110101010111001010010001100110101000010001001001000101000000000"
        );
        assert_eq!(&text[..9], "000000000");
        assert_eq!(&text[text.len() - 9..], "000000000");
        assert_eq!(&text[9..12], "101");
        assert_eq!(&text[54..59], "01010");
        assert_eq!(&text[101..104], "101");
    }

    #[test]
    fn code128_modules() {
        let modules = super::code128_modules("REG").unwrap();
        let text = bits(&modules);
        assert_eq!(&text[..11], "11010010000");
        assert_eq!(&text[11..22], "11000101110");
        assert_eq!(&text[22..33], "10001101000");
        assert_eq!(&text[33..44], "11010001000");
        assert_eq!(&text[44..55], "10110001000");
        assert_eq!(&text[55..], "1100011101011");
    }

    #[test]
    fn qr_roundtrip() {
        let png = render_qr("https://kuprin.su/", 200).unwrap();
        let image = image::load_from_memory(&png).unwrap().into_luma8();
        assert_eq!(image.width(), 200);
        assert_eq!(image.height(), 200);
        let mut prepared = rqrr::PreparedImage::prepare(image);
        let grids = prepared.detect_grids();
        let (_, content) = grids[0].decode().unwrap();
        assert_eq!(content, "https://kuprin.su/");
    }

    #[test]
    fn photo_is_placed() {
        let layout = Layout {
            width: 144,
            height: 72,
            dots_per_point: 1.0,
            areas: vec![Area {
                rect: Rect {
                    start_x: 0.25,
                    start_y: 0.8,
                    end_x: 0.75,
                    end_y: 0.2,
                },
                alignment: Alignment { x: 0, y: 0 },
                rotation: 0,
                color: 0,
                text: String::new(),
                condition: String::new(),
                kind: AreaKind::Photo,
            }],
        };
        let bg = solid([255, 255, 255]);
        let shown = decode(
            &render_png(
                &layout,
                &HashMap::new(),
                100,
                &bg,
                Some(&solid([255, 0, 0])),
            )
            .unwrap(),
        );
        let (start_x, start_y, end_x, end_y) =
            area_px(&layout.areas[0].rect, shown.width(), shown.height());
        let (x, y) = red_pixel(&shown, start_x, start_y, end_x, end_y).expect("red pixel");
        let hidden = decode(&render_png(&layout, &HashMap::new(), 100, &bg, None).unwrap());
        assert_eq!(hidden.get_pixel(x, y).0, [255, 255, 255, 255]);
    }

    fn solid(color: [u8; 3]) -> Vec<u8> {
        let image = RgbaImage::from_pixel(1, 1, Rgba([color[0], color[1], color[2], 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn decode(bytes: &[u8]) -> RgbaImage {
        image::load_from_memory(bytes).unwrap().to_rgba8()
    }

    fn red_pixel(
        image: &RgbaImage,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Option<(u32, u32)> {
        for y in start_y..end_y {
            for x in start_x..end_x {
                if image.get_pixel(x as u32, y as u32).0 == [255, 0, 0, 255] {
                    return Some((x as u32, y as u32));
                }
            }
        }
        None
    }
}
