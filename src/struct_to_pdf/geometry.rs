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

/// `cm` about `(anchor_x, anchor_y)` in PDF points. `skew_x` and `skew_y` are
/// degrees in document space, where y grows downward. The cross terms are
/// negated because PDF y grows upward.
pub(crate) fn frame_matrix(
    anchor_x: f64,
    anchor_y: f64,
    scale_x: f64,
    scale_y: f64,
    skew_x: f64,
    skew_y: f64,
) -> [f64; 6] {
    let shear_x = -skew_x.to_radians().tan();
    let shear_y = -skew_y.to_radians().tan();
    [
        scale_x,
        shear_y,
        shear_x,
        scale_y,
        anchor_x - scale_x * anchor_x - shear_x * anchor_y,
        anchor_y - shear_y * anchor_x - scale_y * anchor_y,
    ]
}

pub fn pdf_point(page: &Page, document_x: f64, document_y: f64) -> (f32, f32) {
    let units = page.page_size.units;
    let x = to_points(page.page_size.bleeds.left, units) + to_points(document_x, units);
    let y = to_points(page.page_size.bleeds.bottom, units)
        + (to_points(page.page_size.height, units) - to_points(document_y, units));
    (x, y)
}
