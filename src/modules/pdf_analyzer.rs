use lopdf::{Document, Object};
use std::collections::HashMap;
use std::io::Read;

use crate::modules::data_structures::*;

pub fn extract_pages(doc: &Document) -> Result<Vec<PageAnalysis>, Box<dyn std::error::Error>> {
    let pages = doc.get_pages();
    let mut page_analyses = Vec::new();

    for (page_num, page_id) in pages.iter() {
        let page_analysis = analyze_page(doc, *page_num as usize, *page_id)?;
        page_analyses.push(page_analysis);
    }

    Ok(page_analyses)
}

pub fn extract_metadata(doc: &Document) -> Result<PdfMetadata, Box<dyn std::error::Error>> {
    let pages = doc.get_pages();
    let page_count = pages.len();

    // Try to get document info
    let mut title = None;
    let mut author = None;
    let mut creator = None;
    let mut producer = None;
    let mut creation_date = None;
    let mut modification_date = None;

    if let Ok(info_id) = doc.trailer.get(b"Info") {
        if let Ok(info_ref) = info_id.as_reference() {
            if let Ok(info_obj) = doc.get_object(info_ref) {
                if let Object::Dictionary(info_dict) = info_obj {
                    title = get_string_from_dict(info_dict, b"Title");
                    author = get_string_from_dict(info_dict, b"Author");
                    creator = get_string_from_dict(info_dict, b"Creator");
                    producer = get_string_from_dict(info_dict, b"Producer");
                    creation_date = get_string_from_dict(info_dict, b"CreationDate");
                    modification_date = get_string_from_dict(info_dict, b"ModDate");
                }
            }
        }
    }

    Ok(PdfMetadata {
        page_count,
        title,
        author,
        creator,
        producer,
        creation_date,
        modification_date,
    })
}

fn analyze_page(
    doc: &Document,
    page_num: usize,
    page_id: (u32, u16),
) -> Result<PageAnalysis, Box<dyn std::error::Error>> {
    let mut resources = PageResources {
        images: HashMap::new(),
        fonts: HashMap::new(),
    };
    let mut contents = Vec::new();
    let mut page_size = PageSize {
        width: 595.92,               // Default A4 width in points
        height: 842.04,              // Default A4 height in points
        units: "points".to_string(), // PDF default unit (1/72 inch)
        media_box: None,
    };

    if let Ok(page_obj) = doc.get_object(page_id) {
        if let Object::Dictionary(page_dict) = page_obj {
            // Extract page size from MediaBox
            if let Ok(mediabox_obj) = page_dict.get(b"MediaBox") {
                page_size = extract_page_size(mediabox_obj, page_dict)?;
            }

            // Extract resources
            if let Ok(resources_obj) = page_dict.get(b"Resources") {
                resources = extract_resources(doc, resources_obj, page_num)?;
            }

            // Extract content boxes
            if let Ok(contents_obj) = page_dict.get(b"Contents") {
                contents = extract_content_boxes(doc, contents_obj, &page_size, &resources)?;
            }
        }
    }

    Ok(PageAnalysis {
        page_number: page_num,
        page_id: format!("{:?}", page_id),
        page_size,
        resources,
        contents,
    })
}

fn extract_page_size(
    mediabox_obj: &Object,
    page_dict: &lopdf::Dictionary,
) -> Result<PageSize, Box<dyn std::error::Error>> {
    match mediabox_obj {
        Object::Array(mediabox_array) => {
            if mediabox_array.len() >= 4 {
                let mut mediabox = [0.0; 4];
                for (i, value) in mediabox_array.iter().enumerate() {
                    if i < 4 {
                        match value {
                            Object::Real(r) => mediabox[i] = *r,
                            Object::Integer(i_val) => mediabox[i] = *i_val as f32,
                            _ => {}
                        }
                    }
                }

                // MediaBox format: [llx, lly, urx, ury] (lower-left and upper-right corners)
                let width = mediabox[2] - mediabox[0]; // urx - llx
                let height = mediabox[3] - mediabox[1]; // ury - lly

                // Check for UserUnit specification (PDF 1.6+)
                let (units, unit_factor) = if let Ok(userunit_obj) = page_dict.get(b"UserUnit") {
                    match userunit_obj {
                        Object::Real(factor) => {
                            if (*factor - 1.0).abs() < 0.001 {
                                ("points".to_string(), 1.0)
                            } else {
                                (format!("points (UserUnit: {})", factor), *factor)
                            }
                        }
                        Object::Integer(factor) => {
                            if *factor == 1 {
                                ("points".to_string(), 1.0)
                            } else {
                                (format!("points (UserUnit: {})", factor), *factor as f32)
                            }
                        }
                        _ => ("points".to_string(), 1.0),
                    }
                } else {
                    ("points".to_string(), 1.0) // Default PDF points
                };

                Ok(PageSize {
                    width: width * unit_factor,
                    height: height * unit_factor,
                    units,
                    media_box: Some(mediabox),
                })
            } else {
                Ok(PageSize {
                    width: 595.92,               // Default A4 width
                    height: 842.04,              // Default A4 height
                    units: "points".to_string(), // PDF default unit (1/72 inch)
                    media_box: None,
                })
            }
        }
        _ => {
            Ok(PageSize {
                width: 595.92,               // Default A4 width
                height: 842.04,              // Default A4 height
                units: "points".to_string(), // PDF default unit (1/72 inch)
                media_box: None,
            })
        }
    }
}

fn extract_resources(
    doc: &Document,
    resources_obj: &Object,
    page_num: usize,
) -> Result<PageResources, Box<dyn std::error::Error>> {
    let mut images = HashMap::new();
    let mut fonts = HashMap::new();

    let resources_dict = if let Ok(resources_ref) = resources_obj.as_reference() {
        if let Ok(resources_obj) = doc.get_object(resources_ref) {
            if let Object::Dictionary(dict) = resources_obj {
                dict
            } else {
                return Ok(PageResources { images, fonts });
            }
        } else {
            return Ok(PageResources { images, fonts });
        }
    } else if let Object::Dictionary(dict) = resources_obj {
        dict
    } else {
        return Ok(PageResources { images, fonts });
    };

    // Extract images from XObject
    if let Ok(xobjects) = resources_dict.get(b"XObject") {
        if let Object::Dictionary(xobj_dict) = xobjects {
            for (name, xobj_ref) in xobj_dict.iter() {
                let name_str = String::from_utf8_lossy(name);
                if let Ok(xobj_id) = xobj_ref.as_reference() {
                    if let Ok(xobj_obj) = doc.get_object(xobj_id) {
                        if let Object::Stream(stream) = xobj_obj {
                            if let Ok(subtype) = stream.dict.get(b"Subtype") {
                                if let Object::Name(subtype_bytes) = subtype {
                                    let subtype_str = String::from_utf8_lossy(subtype_bytes);
                                    if subtype_str == "Image" {
                                        let image_resource = crate::modules::image_extractor::extract_image_resource(
                                                &name_str, stream, page_num
                                            )?;
                                        images.insert(name_str.to_string(), image_resource);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else if let Ok(xobj_ref) = xobjects.as_reference() {
            if let Ok(xobj_obj) = doc.get_object(xobj_ref) {
                if let Object::Dictionary(xobj_dict) = xobj_obj {
                    for (name, xobj_ref) in xobj_dict.iter() {
                        let name_str = String::from_utf8_lossy(name);
                        if let Ok(xobj_id) = xobj_ref.as_reference() {
                            if let Ok(xobj_obj) = doc.get_object(xobj_id) {
                                if let Object::Stream(stream) = xobj_obj {
                                    if let Ok(subtype) = stream.dict.get(b"Subtype") {
                                        if let Object::Name(subtype_bytes) = subtype {
                                            let subtype_str =
                                                String::from_utf8_lossy(subtype_bytes);
                                            if subtype_str == "Image" {
                                                let image_resource = crate::modules::image_extractor::extract_image_resource(
                                                        &name_str, stream, page_num
                                                    )?;
                                                images.insert(name_str.to_string(), image_resource);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Extract fonts
    if let Ok(fonts_obj) = resources_dict.get(b"Font") {
        if let Object::Dictionary(fonts_dict) = fonts_obj {
            for (name, font_ref) in fonts_dict.iter() {
                let name_str = String::from_utf8_lossy(name);
                if let Ok(font_id) = font_ref.as_reference() {
                    if let Ok(font_obj) = doc.get_object(font_id) {
                        if let Object::Dictionary(font_dict) = font_obj {
                            let font_resource = extract_font_resource(&name_str, font_dict, doc)?;
                            fonts.insert(name_str.to_string(), font_resource);
                        }
                    }
                }
            }
        } else if let Ok(fonts_ref) = fonts_obj.as_reference() {
            if let Ok(fonts_obj) = doc.get_object(fonts_ref) {
                if let Object::Dictionary(fonts_dict) = fonts_obj {
                    for (name, font_ref) in fonts_dict.iter() {
                        let name_str = String::from_utf8_lossy(name);
                        if let Ok(font_id) = font_ref.as_reference() {
                            if let Ok(font_obj) = doc.get_object(font_id) {
                                if let Object::Dictionary(font_dict) = font_obj {
                                    let font_resource =
                                        extract_font_resource(&name_str, font_dict, doc)?;
                                    fonts.insert(name_str.to_string(), font_resource);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(PageResources { images, fonts })
}

fn extract_font_resource(
    name: &str,
    font_dict: &lopdf::Dictionary,
    doc: &Document,
) -> Result<FontResource, Box<dyn std::error::Error>> {
    let base_font = get_string_from_dict(font_dict, b"BaseFont");
    let font_type = get_string_from_dict(font_dict, b"Subtype");
    let encoding = get_string_from_dict(font_dict, b"Encoding");

    let mut to_unicode = None;
    if let Ok(tounicode_ref) = font_dict.get(b"ToUnicode") {
        if let Ok(ref_id) = tounicode_ref.as_reference() {
            if let Ok(tounicode_obj) = doc.get_object(ref_id) {
                if let Object::Stream(tounicode_stream) = tounicode_obj {
                    // Try to decompress the ToUnicode stream
                    let mut tounicode_content = tounicode_stream.content.clone();
                    if let Ok(filter) = tounicode_stream.dict.get(b"Filter") {
                        if let Object::Name(filter_bytes) = filter {
                            let filter_str = String::from_utf8_lossy(filter_bytes);
                            if &*filter_str == "FlateDecode" {
                                use flate2::read::ZlibDecoder;
                                let mut decoder = ZlibDecoder::new(&tounicode_stream.content[..]);
                                let mut decompressed = Vec::new();
                                if decoder.read_to_end(&mut decompressed).is_ok() {
                                    tounicode_content = decompressed;
                                }
                            }
                        }
                    }
                    to_unicode = Some(String::from_utf8_lossy(&tounicode_content).to_string());
                }
            }
        }
    }

    let description = format!(
        "Font: {} ({})",
        base_font.as_deref().unwrap_or("Unknown"),
        font_type.as_deref().unwrap_or("Unknown")
    );

    Ok(FontResource {
        name: name.to_string(),
        base_font,
        font_type,
        encoding,
        to_unicode,
        extracted_path: None, // Fonts are not extracted to files in this implementation
        description,
    })
}

fn extract_content_boxes(
    doc: &Document,
    contents_obj: &Object,
    page_size: &PageSize,
    resources: &PageResources,
) -> Result<Vec<ContentBox>, Box<dyn std::error::Error>> {
    let mut content_boxes = Vec::new();

    // Initialize viewport transformation matrix
    // -1 on scale factor for Y direction to flip the viewport vertically, as PDF uses bottom-left coordinate system
    // page_size.height for linear shift is a position of the viewport bottom edge on the page
    let viewport_matrix: [f32; 6] = [1.0, 0.0, 0.0, -1.0, 0.0, page_size.height];

    // Handle different content object types
    match contents_obj {
        Object::Array(contents_array) => {
            // Multiple content streams
            for content_item in contents_array {
                if let Ok(content_ref) = content_item.as_reference() {
                    let mut boxes =
                        analyze_content_stream(doc, &viewport_matrix, content_ref, resources)?;
                    content_boxes.append(&mut boxes);
                }
            }
        }
        _ => {
            // Single content stream
            if let Ok(content_ref) = contents_obj.as_reference() {
                let mut boxes =
                    analyze_content_stream(doc, &viewport_matrix, content_ref, resources)?;
                content_boxes.append(&mut boxes);
            }
        }
    }

    Ok(content_boxes)
}

fn analyze_content_stream(
    doc: &Document,
    viewport_matrix: &[f32; 6],
    content_id: (u32, u16),
    resources: &PageResources,
) -> Result<Vec<ContentBox>, Box<dyn std::error::Error>> {
    let mut content_boxes = Vec::new();

    if let Ok(content_obj) = doc.get_object(content_id) {
        if let Object::Stream(stream) = content_obj {
            // Decompress the content stream
            let raw_content = get_decompressed_content(stream);

            // Parse the content
            if let Ok(decoded) = lopdf::content::Content::decode(&raw_content) {
                content_boxes = extract_content_from_operations(
                    &decoded.operations,
                    &viewport_matrix,
                    resources,
                )?;
            }
        }
    }

    Ok(content_boxes)
}

fn extract_content_from_operations(
    operations: &[lopdf::content::Operation],
    viewport_matrix: &[f32; 6],
    resources: &PageResources,
) -> Result<Vec<ContentBox>, Box<dyn std::error::Error>> {
    let mut content_boxes = Vec::new();

    let mut transformation_matrix = viewport_matrix.clone();

    println!("=============== RESTART ===============");
    println!("Transformation matrix RESTART: {:?}", transformation_matrix);
    // Extract text content using existing text analyzer logic
    let text_content =
        crate::modules::text_analyzer::extract_text_content_from_operations(operations)?;

    println!(
        "Transformation matrix AFTER TEXT: {:?}",
        transformation_matrix
    );

    // Extract image content using existing positioning logic
    let image_content = crate::modules::image_extractor::extract_image_content_from_operations(
        operations,
        viewport_matrix,
        resources,
        &mut transformation_matrix,
    )?;

    println!(
        "Transformation matrix AFTER IMAGE: {:?}",
        transformation_matrix
    );

    content_boxes.extend(text_content);
    content_boxes.extend(image_content);

    Ok(content_boxes)
}

fn get_decompressed_content(stream: &lopdf::Stream) -> Vec<u8> {
    if let Ok(filter) = stream.dict.get(b"Filter") {
        if let Object::Name(filter_bytes) = filter {
            let filter_str = String::from_utf8_lossy(filter_bytes);
            if &*filter_str == "FlateDecode" {
                use flate2::read::ZlibDecoder;
                use std::io::Read;
                let mut decoder = ZlibDecoder::new(&stream.content[..]);
                let mut decompressed = Vec::new();
                if decoder.read_to_end(&mut decompressed).is_ok() {
                    decompressed
                } else {
                    stream.content.clone()
                }
            } else {
                stream.content.clone()
            }
        } else {
            stream.content.clone()
        }
    } else {
        stream.content.clone()
    }
}

fn get_string_from_dict(dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
    if let Ok(value) = dict.get(key) {
        match value {
            Object::String(bytes, _) => String::from_utf8(bytes.clone()).ok(),
            Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
            _ => None,
        }
    } else {
        None
    }
}
