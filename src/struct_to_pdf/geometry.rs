use crate::struct_to_pdf::{Page, Units};

pub fn to_points(value: f64, units: Units) -> f32 {
    match units {
        Units::Points => value as f32,
        Units::Mm => (value * 72.0 / 25.4) as f32,
    }
}

pub fn media_size(page: &Page) -> (f32, f32) {
    let units = page.page_size.units;
    let bleeds = page.page_size.bleeds;
    let width = to_points(page.page_size.width + bleeds.left + bleeds.right, units);
    let height = to_points(page.page_size.height + bleeds.bottom + bleeds.top, units);
    (width, height)
}

pub fn media_box(page: &Page) -> [f32; 4] {
    let (width, height) = media_size(page);
    [0.0, 0.0, width, height]
}

pub fn trim_box(page: &Page) -> [f32; 4] {
    let units = page.page_size.units;
    let bleeds = page.page_size.bleeds;
    let left = to_points(bleeds.left, units);
    let bottom = to_points(bleeds.bottom, units);
    let width = to_points(page.page_size.width, units);
    let height = to_points(page.page_size.height, units);
    [left, bottom, left + width, bottom + height]
}

pub fn pdf_point(page: &Page, document_x: f64, document_y: f64) -> (f32, f32) {
    let units = page.page_size.units;
    let x = to_points(page.page_size.bleeds.left, units) + to_points(document_x, units);
    let y = to_points(page.page_size.bleeds.bottom, units)
        + (to_points(page.page_size.height, units) - to_points(document_y, units));
    (x, y)
}
