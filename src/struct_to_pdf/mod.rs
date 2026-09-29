mod fonts;
mod geometry;
mod images;
mod markup;
mod page;
mod text;

use std::path::Path;

use lopdf::{dictionary, Document, Object, Stream};

pub use geometry::{media_box, media_size, pdf_point, to_points, trim_box};
pub use page::{
    load_page, parse_page, Bleeds, ContentEntry, FontResource, FontStyle, HorizontalAlign,
    ImagePlacement, ImageResource, Padding, Page, PageSize, RenderError, SourceFormat, TextBody,
    TextBox, Units, VerticalAlign,
};
pub use text::authored_text_bounds;

pub fn render_page(page: &Page, base_dir: &Path) -> Result<Vec<u8>, RenderError> {
    let painted = images::paint_contents(page, base_dir)?;

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut xobjects = lopdf::Dictionary::new();
    for (name, stream) in painted.xobjects {
        let image_id = doc.add_object(stream);
        xobjects.set(name, Object::Reference(image_id));
    }
    let mut font_dict = lopdf::Dictionary::new();
    for pending in painted.fonts {
        let font_id = fonts::embed(&mut doc, &pending.bytes, &pending.glyphs)?;
        font_dict.set(pending.resource_name, Object::Reference(font_id));
    }
    let mut resources = lopdf::Dictionary::new();
    if !xobjects.is_empty() {
        resources.set("XObject", xobjects);
    }
    if !font_dict.is_empty() {
        resources.set("Font", font_dict);
    }
    let content_id = doc.add_object(Stream::new(dictionary! {}, painted.content));
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
    doc.set_object(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
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
