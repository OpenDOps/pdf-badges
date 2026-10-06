mod barcode;
pub mod ffi;
mod fonts;
mod geometry;
mod images;
mod markup;
mod page;
mod raster;
mod template;
mod text;

use std::collections::HashMap;
use std::path::Path;

use lopdf::{dictionary, Document, Object, ObjectId, Stream};

pub use geometry::{media_box, media_size, pdf_point, to_points, trim_box};
pub use page::{
    load_page, parse_page, Barcode, Bleeds, ContentEntry, FontResource, FontStyle, HorizontalAlign,
    ImagePlacement, ImageResource, Padding, Page, PageSize, Photo, RenderError, SourceFormat,
    TextBody, TextBox, Units, VerticalAlign,
};
pub(crate) use raster::{bmp_bytes, png_bytes, render_pixels, FaceSet};
pub use raster::{render, Raster};
pub use template::{
    fill, fill_raw, index_template, scan, Hole, IndexedBox, TemplateField, TemplateIndex,
};
pub use text::authored_text_bounds;

pub fn render_rows(
    page: &Page,
    index: &TemplateIndex,
    rows: &[HashMap<String, String>],
    base_dir: &Path,
) -> Result<Vec<u8>, RenderError> {
    if rows.is_empty() {
        return Err(RenderError {
            context: "page".to_string(),
            message: "no rows".to_string(),
        });
    }
    for row in rows {
        for key in row.keys() {
            if index.fields.iter().all(|field| field.name != *key) {
                return Err(RenderError {
                    context: "page".to_string(),
                    message: format!("unknown field {key}"),
                });
            }
        }
    }
    let mut filled = Vec::with_capacity(rows.len());
    for row in rows {
        let mut clone = page.clone();
        for indexed in &index.boxes {
            let Some(text) = text_box_mut(&mut clone, &indexed.box_id) else {
                return Err(RenderError {
                    context: indexed.box_id.clone(),
                    message: format!("missing box {}", indexed.box_id),
                });
            };
            text.text.content = Some(fill(indexed, row));
        }
        filled.push(clone);
    }
    render_pages(&filled, base_dir)
}

fn text_box_mut<'a>(page: &'a mut Page, id: &str) -> Option<&'a mut TextBox> {
    for entry in &mut page.contents {
        if let ContentEntry::Text(text) = entry {
            if text.id == id {
                return Some(text);
            }
        }
    }
    None
}

pub fn prepare_visitor(
    page: &Page,
    values: &HashMap<String, String>,
    photos: &HashMap<String, Vec<u8>>,
    base_dir: &Path,
) -> Result<Page, RenderError> {
    let mut page = page.clone();
    page.contents
        .retain(|entry| entry_shown(if_st(entry), values));
    let index = index_template(&page)?;
    for indexed in &index.boxes {
        if let Some(text) = text_box_mut(&mut page, &indexed.box_id) {
            text.text.content = Some(fill(indexed, values));
        } else if let Some(code) = barcode_mut(&mut page, &indexed.box_id) {
            code.content = Some(fill_raw(indexed, values));
        }
    }
    for entry in &mut page.contents {
        if let ContentEntry::Photo(photo) = entry {
            resolve_photo(photo, values, photos, base_dir)?;
        }
    }
    Ok(page)
}

pub fn render_visitor(
    page: &Page,
    values: &HashMap<String, String>,
    photos: &HashMap<String, Vec<u8>>,
    base_dir: &Path,
) -> Result<Vec<u8>, RenderError> {
    let page = prepare_visitor(page, values, photos, base_dir)?;
    render_page(&page, base_dir)
}

fn if_st(entry: &ContentEntry) -> &str {
    match entry {
        ContentEntry::Text(text) => &text.if_st,
        ContentEntry::Image(image) => &image.if_st,
        ContentEntry::Barcode(code) => &code.if_st,
        ContentEntry::Photo(photo) => &photo.if_st,
    }
}

fn entry_shown(if_st: &str, values: &HashMap<String, String>) -> bool {
    if if_st.is_empty() {
        return true;
    }
    match values.get(if_st).map(String::as_str) {
        Some(value) if !value.is_empty() && value != "false" && value != "0" => true,
        _ => false,
    }
}

fn barcode_mut<'a>(page: &'a mut Page, id: &str) -> Option<&'a mut Barcode> {
    for entry in &mut page.contents {
        if let ContentEntry::Barcode(code) = entry {
            if code.id == id {
                return Some(code);
            }
        }
    }
    None
}

fn resolve_photo(
    photo: &mut Photo,
    values: &HashMap<String, String>,
    photos: &HashMap<String, Vec<u8>>,
    base_dir: &Path,
) -> Result<(), RenderError> {
    if let Some(bytes) = photos.get(&photo.field) {
        photo.bytes = (!bytes.is_empty()).then(|| bytes.clone());
        return Ok(());
    }
    let Some(path) = values.get(&photo.field).filter(|path| !path.is_empty()) else {
        photo.bytes = None;
        return Ok(());
    };
    let file = base_dir.join(path);
    photo.bytes = std::fs::read(&file).ok();
    Ok(())
}

pub fn render_page(page: &Page, base_dir: &Path) -> Result<Vec<u8>, RenderError> {
    render_pages(std::slice::from_ref(page), base_dir)
}

pub fn render_pages(pages: &[Page], base_dir: &Path) -> Result<Vec<u8>, RenderError> {
    if pages.is_empty() {
        return Err(RenderError {
            context: "page".to_string(),
            message: "no pages".to_string(),
        });
    }

    let painted = pages
        .iter()
        .map(|page| images::paint_contents(page, base_dir))
        .collect::<Result<Vec<_>, _>>()?;

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_groups = union_fonts(&painted);
    let mut font_ids = Vec::with_capacity(font_groups.len());
    for group in &font_groups {
        font_ids.push(fonts::embed(&mut doc, &group.bytes, &group.glyphs)?);
    }

    let mut shared_images: Vec<(Vec<u8>, ObjectId)> = Vec::new();
    let mut kids = Vec::with_capacity(pages.len());
    for (page, painted_page) in pages.iter().zip(painted) {
        let mut xobjects = lopdf::Dictionary::new();
        for (name, stream) in painted_page.xobjects {
            let image_id = shared_image(&mut doc, &mut shared_images, stream);
            xobjects.set(name, Object::Reference(image_id));
        }
        let mut font_dict = lopdf::Dictionary::new();
        for pending in painted_page.fonts {
            let index = font_groups
                .iter()
                .position(|group| group.bytes == pending.bytes)
                .expect("painted font belongs to a group");
            font_dict.set(pending.resource_name, Object::Reference(font_ids[index]));
        }
        let mut resources = lopdf::Dictionary::new();
        if !xobjects.is_empty() {
            resources.set("XObject", xobjects);
        }
        if !font_dict.is_empty() {
            resources.set("Font", font_dict);
        }
        let content_id = doc.add_object(Stream::new(dictionary! {}, painted_page.content));
        let media = media_box(page);
        let trim = trim_box(page);
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => pdf_rect(media),
            "BleedBox" => pdf_rect(media),
            "CropBox" => pdf_rect(media),
            "TrimBox" => pdf_rect(trim),
            "Contents" => content_id,
            "Resources" => resources,
        });
        kids.push(Object::Reference(page_id));
    }

    doc.set_object(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => pages.len() as i64,
        },
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|err| RenderError {
        context: "page".to_string(),
        message: err.to_string(),
    })?;
    Ok(bytes)
}

struct FontGroup {
    bytes: Vec<u8>,
    glyphs: Vec<(u16, u32)>,
}

fn union_fonts(painted: &[images::PaintedPage]) -> Vec<FontGroup> {
    let mut groups = Vec::new();
    for page in painted {
        for font in &page.fonts {
            let index = if let Some(index) = groups
                .iter()
                .position(|group: &FontGroup| group.bytes == font.bytes)
            {
                index
            } else {
                groups.push(FontGroup {
                    bytes: font.bytes.clone(),
                    glyphs: Vec::new(),
                });
                groups.len() - 1
            };
            for &(gid, unicode) in &font.glyphs {
                if groups[index].glyphs.iter().all(|(id, _)| *id != gid) {
                    groups[index].glyphs.push((gid, unicode));
                }
            }
        }
    }
    groups
}

fn shared_image(
    doc: &mut Document,
    images: &mut Vec<(Vec<u8>, ObjectId)>,
    stream: Stream,
) -> ObjectId {
    if let Some((_, id)) = images.iter().find(|(bytes, _)| bytes == &stream.content) {
        return *id;
    }
    let bytes = stream.content.clone();
    let id = doc.add_object(stream);
    images.push((bytes, id));
    id
}

pub fn save_pdf(bytes: &[u8], path: &Path) -> Result<(), RenderError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| RenderError {
                context: "page".to_string(),
                message: err.to_string(),
            })?;
        }
    }
    std::fs::write(path, bytes).map_err(|err| RenderError {
        context: "page".to_string(),
        message: err.to_string(),
    })
}

fn pdf_rect(rect: [f32; 4]) -> Object {
    Object::Array(rect.into_iter().map(Object::Real).collect())
}
