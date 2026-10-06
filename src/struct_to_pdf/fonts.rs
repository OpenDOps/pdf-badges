use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::struct_to_pdf::RenderError;

pub struct LoadedFont {
    pub bytes: Vec<u8>,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
}

pub struct GlyphRun {
    pub encoded: Vec<u8>,
    pub glyphs: Vec<(u16, u32)>,
}

pub fn load_font(path: &Path, context: &str) -> Result<LoadedFont, RenderError> {
    let bytes = fs::read(path).map_err(|_| RenderError {
        context: context.to_string(),
        message: format!("font file not found: {}", path.display()),
    })?;
    load_font_bytes(bytes, context)
}

pub fn load_font_bytes(bytes: Vec<u8>, context: &str) -> Result<LoadedFont, RenderError> {
    let face = ttf_parser::Face::parse(&bytes, 0).map_err(|err| RenderError {
        context: context.to_string(),
        message: format!("font file is not a valid TTF: {err}"),
    })?;
    let units_per_em = face.units_per_em();
    let ascender = face.ascender();
    let descender = face.descender();
    Ok(LoadedFont {
        bytes,
        units_per_em,
        ascender,
        descender,
    })
}

pub fn ascent_in_page_units(font: &LoadedFont, font_size: f64) -> f64 {
    font_size * f64::from(font.ascender) / f64::from(font.units_per_em)
}

pub fn descent_in_page_units(font: &LoadedFont, font_size: f64) -> f64 {
    font_size * f64::from(-font.descender) / f64::from(font.units_per_em)
}

pub fn encode_glyphs(
    glyphs: &std::collections::HashMap<char, (u16, u16)>,
    text: &str,
    context: &str,
) -> Result<GlyphRun, RenderError> {
    let mut encoded = Vec::new();
    let mut used = Vec::new();
    let mut missing = Vec::new();
    for ch in text.chars() {
        match glyphs.get(&ch) {
            Some((gid, _)) => {
                encoded.extend_from_slice(&gid.to_be_bytes());
                used.push((*gid, ch as u32));
            }
            None => missing.push(ch),
        }
    }
    if !missing.is_empty() {
        return Err(uncovered(context, &missing));
    }
    Ok(GlyphRun {
        encoded,
        glyphs: used,
    })
}

pub fn uncovered(context: &str, missing: &[char]) -> RenderError {
    let listed = missing
        .iter()
        .map(|ch| format!("U+{:04X} ({ch})", u32::from(*ch)))
        .collect::<Vec<_>>()
        .join(", ");
    log::warn!("box {context}: font does not cover {listed}");
    RenderError {
        context: context.to_string(),
        message: format!("font does not cover {listed}"),
    }
}

pub fn embed(
    doc: &mut Document,
    bytes: &[u8],
    glyphs: &[(u16, u32)],
) -> Result<ObjectId, RenderError> {
    let face = ttf_parser::Face::parse(bytes, 0).map_err(|err| RenderError {
        context: "page".to_string(),
        message: format!("font file is not a valid TTF: {err}"),
    })?;
    let units = f64::from(face.units_per_em());
    let scale = 1000.0 / units;
    let cmap_id = doc.add_object(Stream::new(dictionary! {}, to_unicode(glyphs)));
    let file_id = doc.add_object(
        Stream::new(
            dictionary! {
                "Length1" => bytes.len() as i64,
            },
            bytes.to_vec(),
        )
        .with_compression(false),
    );
    let bbox = face.global_bounding_box();
    let descriptor_id = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "F",
        "Flags" => 32,
        "FontBBox" => vec![
            Object::Integer((f64::from(bbox.x_min) * scale).round() as i64),
            Object::Integer((f64::from(bbox.y_min) * scale).round() as i64),
            Object::Integer((f64::from(bbox.x_max) * scale).round() as i64),
            Object::Integer((f64::from(bbox.y_max) * scale).round() as i64),
        ],
        "ItalicAngle" => 0,
        "Ascent" => (f64::from(face.ascender()) * scale).round() as i64,
        "Descent" => (f64::from(face.descender()) * scale).round() as i64,
        "CapHeight" => (f64::from(face.ascender()) * scale).round() as i64,
        "StemV" => 80,
        "FontFile2" => Object::Reference(file_id),
    });

    let mut widths = Vec::new();
    let mut seen = BTreeMap::new();
    for (gid, _) in glyphs {
        seen.insert(*gid, ());
    }
    for gid in seen.keys() {
        let advance = face
            .glyph_hor_advance(ttf_parser::GlyphId(*gid))
            .unwrap_or(0);
        let width = (f64::from(advance) * scale).round() as i64;
        widths.push(Object::Integer(i64::from(*gid)));
        widths.push(Object::Array(vec![Object::Integer(width)]));
    }

    let mut cid_system = lopdf::Dictionary::new();
    cid_system.set(
        "Registry",
        Object::String(b"Adobe".to_vec(), StringFormat::Literal),
    );
    cid_system.set(
        "Ordering",
        Object::String(b"Identity".to_vec(), StringFormat::Literal),
    );
    cid_system.set("Supplement", 0i64);

    let cid_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => "F",
        "CIDSystemInfo" => cid_system,
        "FontDescriptor" => Object::Reference(descriptor_id),
        "CIDToGIDMap" => "Identity",
        "DW" => 1000,
        "W" => widths,
    });
    let type0_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "F",
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(cid_id)],
        "ToUnicode" => Object::Reference(cmap_id),
    });
    Ok(type0_id)
}

fn to_unicode(glyphs: &[(u16, u32)]) -> Vec<u8> {
    let mut unique = BTreeMap::new();
    for (gid, unicode) in glyphs {
        unique.entry(*gid).or_insert(*unicode);
    }
    let entries: Vec<(u16, u32)> = unique.into_iter().collect();
    let mut body = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo\n\
         << /Registry (Adobe)\n\
         /Ordering (UCS)\n\
         /Supplement 0\n\
         >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n",
    );
    for chunk in entries.chunks(100) {
        body.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, unicode) in chunk {
            body.push_str(&format!("<{gid:04X}> {}\n", unicode_dest(*unicode)));
        }
        body.push_str("endbfchar\n");
    }
    body.push_str(
        "endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n",
    );
    body.into_bytes()
}

fn unicode_dest(unicode: u32) -> String {
    let Some(ch) = char::from_u32(unicode) else {
        return "<003F>".to_string();
    };
    let mut encoded = [0u16; 2];
    let pair = ch.encode_utf16(&mut encoded);
    if pair.len() == 1 {
        format!("<{:04X}>", pair[0])
    } else {
        format!("<{:04X}{:04X}>", pair[0], pair[1])
    }
}
