/// Column letters.

const MAX_COLUMN: u32 = 16384; // XFD, the last Excel column

/// Map a 0-based column index to the sheet letter. 0 is A, 26 is AA.
pub fn excel_id(index: u32) -> String {
    let mut number = index + 1;
    let mut letters = Vec::new();
    while number > 0 {
        number -= 1;
        letters.push(b'A' + (number % 26) as u8);
        number /= 26;
    }
    letters.reverse();
    String::from_utf8(letters).expect("column letters are ASCII")
}

/// Map a sheet letter to a 0-based index. `A` and `a` are 0. Anything else is None.
pub fn parse_excel_id(text: &str) -> Option<u32> {
    let letters = text.trim();
    if letters.is_empty()
        || !letters.is_ascii()
        || !letters.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return None;
    }
    let mut number: u32 = 0;
    for byte in letters.bytes() {
        let letter = byte.to_ascii_uppercase();
        number = number
            .checked_mul(26)?
            .checked_add(u32::from(letter - b'A') + 1)?;
        if number > MAX_COLUMN {
            return None;
        }
    }
    Some(number - 1)
}
