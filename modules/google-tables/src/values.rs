/// Cell classification.
use crate::model::{CellValue, DateValue};

const INT64_MIN: i64 = i64::MIN;
const INT64_MAX: i64 = i64::MAX;

/// One cell as the sheet reports it, before classification.
#[derive(Debug, Clone)]
pub struct Reading {
    pub kind: String,
    pub display: String,
    pub serial: f64,
    pub error_code: i32,
    pub is_date: bool,
    pub is_time: bool,
    pub is_logical: bool,
    pub is_percent: bool,
    pub is_currency: bool,
    pub null_year: i32,
    pub null_month: u32,
    pub null_day: u32,
}

impl Reading {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            display: String::new(),
            serial: 0.0,
            error_code: 0,
            is_date: false,
            is_time: false,
            is_logical: false,
            is_percent: false,
            is_currency: false,
            null_year: 1899,
            null_month: 12,
            null_day: 30,
        }
    }
}

pub fn classify(reading: &Reading) -> Option<CellValue> {
    if reading.kind == "empty" {
        return None;
    }
    if reading.kind == "formula" && reading.display.is_empty() && reading.error_code == 0 {
        return None;
    }
    if reading.kind == "error" || (reading.kind == "formula" && reading.error_code != 0) {
        return Some(CellValue::Text(reading.display.clone()));
    }
    if reading.kind == "text" {
        return Some(CellValue::Text(reading.display.clone()));
    }
    if reading.is_date || reading.is_time {
        return Some(CellValue::Date(date_value(reading)));
    }
    if reading.is_logical {
        return Some(CellValue::Bool(reading.serial != 0.0));
    }
    if reading.is_percent || reading.is_currency {
        return Some(CellValue::Float(reading.serial));
    }
    let nearest = reading.serial.round();
    if (reading.serial - nearest).abs() < 1e-9
        && (INT64_MIN as f64) <= nearest
        && nearest <= (INT64_MAX as f64)
    {
        return Some(CellValue::Int(nearest as i64));
    }
    Some(CellValue::Float(reading.serial))
}

fn date_value(reading: &Reading) -> DateValue {
    let mut days = reading.serial.trunc() as i64;
    let has_time = reading.is_time;
    let seconds = if has_time {
        let seconds = ((reading.serial - (days as f64)) * 86400.0).round() as i64;
        if seconds >= 86400 {
            days += 1;
            0
        } else if seconds < 0 {
            days -= 1;
            seconds + 86400
        } else {
            seconds
        }
    } else {
        0
    };
    let epoch = days_from_civil(
        reading.null_year,
        reading.null_month as i32,
        reading.null_day as i32,
    );
    let (year, month, day) = civil_from_days(epoch + days);
    let hour = seconds.div_euclid(3600);
    let remainder = seconds.rem_euclid(3600);
    let minute = remainder.div_euclid(60);
    let second = remainder.rem_euclid(60);
    DateValue {
        year,
        month: month as u32,
        day: day as u32,
        hour: hour as u32,
        minute: minute as u32,
        second: second as u32,
        has_time,
    }
}

/// Days since 1970-01-01 for a civil date. Matches the Howard Hinnant algorithm.
fn days_from_civil(mut year: i32, month: i32, day: i32) -> i64 {
    year -= i32::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = (year - era * 400) as i64;
    let month_shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shifted + 2) / 5 + day - 1;
    let day_of_era = yoe * 365 + yoe / 4 - yoe / 100 + i64::from(day_of_year);
    i64::from(era) * 146097 + day_of_era - 719468
}

fn civil_from_days(days: i64) -> (i32, i32, i32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let day_of_era = (z - era * 146097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era as i64 + i64::from(era) * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as i32, day as i32)
}
