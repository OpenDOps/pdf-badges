use qrcode::{Color, EcLevel, QrCode};

use crate::struct_to_pdf::RenderError;

pub enum Symbol {
    Bars(Vec<bool>),
    Qr { width: usize, modules: Vec<bool> },
}

pub fn symbol(kind: &str, text: &str, id: &str) -> Result<Symbol, RenderError> {
    match kind {
        "ean13" => Ok(Symbol::Bars(ean13(text, id)?)),
        "code128" => Ok(Symbol::Bars(code128(text, id)?)),
        "qr" => {
            let (width, modules) = qr(text, id)?;
            Ok(Symbol::Qr { width, modules })
        }
        _ => Err(error(id, &format!("unknown barcode type {kind}"))),
    }
}

fn ean13(code: &str, id: &str) -> Result<Vec<bool>, RenderError> {
    if code.len() != 13 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(id, &format!("ean13 {code}")));
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

fn code128(code: &str, id: &str) -> Result<Vec<bool>, RenderError> {
    let mut values = Vec::new();
    for ch in code.chars() {
        let byte = ch as u32;
        if !(32..127).contains(&byte) {
            return Err(error(id, &format!("code 128 {code}")));
        }
        values.push((byte - 32) as usize);
    }
    if values.is_empty() {
        return Err(error(id, "code 128"));
    }
    let mut sum = 104;
    for (index, value) in values.iter().enumerate() {
        sum += (index + 1) * value;
    }
    let check = sum % 103;
    let mut bits = Vec::new();
    let mut black = true;
    push(&mut bits, &CODE128[104], &mut black);
    for value in values {
        push(&mut bits, &CODE128[value], &mut black);
    }
    push(&mut bits, &CODE128[check], &mut black);
    push(&mut bits, &[2, 3, 3, 1, 1, 1, 2], &mut black);
    Ok(bits)
}

fn qr(text: &str, id: &str) -> Result<(usize, Vec<bool>), RenderError> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::H)
        .map_err(|err| error(id, &err.to_string()))?;
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

fn error(id: &str, message: &str) -> RenderError {
    RenderError {
        context: id.to_string(),
        message: message.to_string(),
    }
}

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
