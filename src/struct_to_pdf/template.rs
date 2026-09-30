use std::collections::HashMap;

use crate::struct_to_pdf::{ContentEntry, Page, RenderError, TextBox};

#[derive(Debug, Clone, PartialEq)]
pub struct Hole {
    pub name: String,
    pub from: usize,
    pub to: usize,
}

pub fn scan(source: &str, open: &str, close: &str, box_id: &str) -> Result<Vec<Hole>, RenderError> {
    if open.is_empty() || close.is_empty() {
        return Err(scan_error(box_id, "empty delimiter"));
    }
    let mut index = 0;
    let mut holes = Vec::new();
    while index < source.len() {
        let rest = &source[index..];
        if rest.starts_with(open) {
            let start = index;
            let inner_start = index + open.len();
            let Some(relative_close) = source[inner_start..].find(close) else {
                return Err(scan_error(box_id, "unclosed field"));
            };
            let close_at = inner_start + relative_close;
            let inner = &source[inner_start..close_at];
            if inner.contains(open) {
                return Err(scan_error(box_id, "nested field"));
            }
            let name = inner.trim();
            if name.is_empty() {
                return Err(scan_error(box_id, "empty field"));
            }
            if name.starts_with(['#', '/', '^', '!', '>', '&']) {
                return Err(scan_error(box_id, "unsupported tag"));
            }
            holes.push(Hole {
                name: name.to_string(),
                from: start,
                to: close_at + close.len(),
            });
            index = close_at + close.len();
        } else {
            let Some(ch) = rest.chars().next() else {
                break;
            };
            index += ch.len_utf8();
        }
    }
    Ok(holes)
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexedBox {
    pub box_id: String,
    pub source: String,
    pub holes: Vec<Hole>,
}

impl IndexedBox {
    pub fn spans(&self, name: &str) -> Vec<(usize, usize)> {
        self.holes
            .iter()
            .filter(|hole| hole.name == name)
            .map(|hole| (hole.from, hole.to))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateField {
    pub name: String,
    pub box_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateIndex {
    pub boxes: Vec<IndexedBox>,
    pub fields: Vec<TemplateField>,
}

pub fn index_template(page: &Page) -> Result<TemplateIndex, RenderError> {
    let mut boxes = Vec::new();
    let mut fields = Vec::new();
    for entry in &page.contents {
        let ContentEntry::Text(text) = entry else {
            continue;
        };
        if !text.template {
            continue;
        }
        let source = template_source(text)?;
        let holes = scan(
            &source,
            &text.delimiter_open,
            &text.delimiter_close,
            &text.id,
        )?;
        for hole in &holes {
            record_field(&mut fields, &hole.name, &text.id);
        }
        boxes.push(IndexedBox {
            box_id: text.id.clone(),
            source,
            holes,
        });
    }
    Ok(TemplateIndex { boxes, fields })
}

pub fn fill(indexed: &IndexedBox, values: &HashMap<String, String>) -> String {
    let mut capacity = indexed.source.len();
    for hole in &indexed.holes {
        capacity -= hole.to - hole.from;
        if let Some(value) = values.get(&hole.name) {
            capacity += escaped_len(value);
        }
    }
    let mut out = String::with_capacity(capacity);
    let mut cursor = 0;
    for hole in &indexed.holes {
        out.push_str(&indexed.source[cursor..hole.from]);
        if let Some(value) = values.get(&hole.name) {
            push_escaped(&mut out, value);
        }
        cursor = hole.to;
    }
    out.push_str(&indexed.source[cursor..]);
    out
}

fn escaped_len(value: &str) -> usize {
    value
        .chars()
        .map(|ch| match ch {
            '&' => "&amp;".len(),
            '<' => "&lt;".len(),
            '>' => "&gt;".len(),
            _ => ch.len_utf8(),
        })
        .sum()
}

fn push_escaped(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn template_source(text: &TextBox) -> Result<String, RenderError> {
    if let Some(content) = &text.text.content {
        return Ok(content.clone());
    }
    if let Some(preentered) = &text.text.preentered {
        return Ok(preentered.clone());
    }
    Err(RenderError {
        context: text.id.clone(),
        message: "preentered and content are both missing".to_string(),
    })
}

fn record_field(fields: &mut Vec<TemplateField>, name: &str, box_id: &str) {
    if let Some(field) = fields.iter_mut().find(|field| field.name == name) {
        if !field.box_ids.iter().any(|id| id == box_id) {
            field.box_ids.push(box_id.to_string());
        }
        return;
    }
    fields.push(TemplateField {
        name: name.to_string(),
        box_ids: vec![box_id.to_string()],
    });
}

fn scan_error(box_id: &str, message: &str) -> RenderError {
    RenderError {
        context: box_id.to_string(),
        message: message.to_string(),
    }
}
