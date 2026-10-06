use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use lopdf::content::Operation;
use lopdf::{Object, StringFormat};
use ttf_parser::Face;

use crate::struct_to_pdf::fonts::{self, LoadedFont};
use crate::struct_to_pdf::geometry::{pdf_point, to_points};
use crate::struct_to_pdf::markup::{self, LineMetrics, MarkupItem, StyledRun};
use crate::struct_to_pdf::{HorizontalAlign, Page, RenderError, TextBox, Units, VerticalAlign};

pub struct PendingFont {
    pub resource_name: String,
    pub bytes: Vec<u8>,
    pub glyphs: Vec<(u16, u32)>,
}

pub struct DrawnText {
    pub operations: Vec<Operation>,
    pub fonts: Vec<PendingFont>,
}

#[derive(Clone)]
struct Word {
    text: String,
    font_key: String,
    font_size: f64,
    leading: f64,
    line_height: f64,
    underline: bool,
    width_pt: f32,
    space_pt: f32,
}

struct Segment {
    text: String,
    font_key: String,
    font_size: f64,
    underline: bool,
    width_pt: f32,
}

struct LaidLine {
    segments: Vec<Segment>,
    width_pt: f32,
    line_height: f64,
    leading: f64,
    font_key: String,
    font_size: f64,
}

struct UsedFont {
    loaded: LoadedFont,
    resource_name: Option<String>,
    glyphs: Vec<(u16, u32)>,
    seen_glyphs: HashSet<u16>,
}

pub fn draw_text(
    page: &Page,
    text: &TextBox,
    base_dir: &Path,
    font_index: &mut usize,
) -> Result<DrawnText, RenderError> {
    let shown = chosen_text(text)?;
    if shown.is_empty() {
        return Ok(DrawnText {
            operations: Vec::new(),
            fonts: Vec::new(),
        });
    }

    let mut parsed = markup::parse(shown, text)?;
    let mut fonts = load_fonts(page, text, base_dir, &parsed)?;
    let advances = build_advances(&fonts, &parsed.items, &text.id)?;
    let units = page.page_size.units;
    let (content_width, _) = layout_size(text);
    let lines = if text.auto_scale {
        fit_font_sizes(page, text, &mut parsed, &advances)?
    } else {
        layout(
            &parsed.items,
            &parsed.end_metrics,
            &advances,
            to_points(content_width, units),
            units,
            &text.id,
        )?
    };
    if lines.is_empty() {
        return Ok(DrawnText {
            operations: Vec::new(),
            fonts: Vec::new(),
        });
    }

    let operations = paint_lines(page, text, &mut fonts, &advances, font_index, &lines)?;
    let pending = fonts
        .into_iter()
        .filter_map(|(_, used)| {
            let resource_name = used.resource_name?;
            if used.glyphs.is_empty() {
                return None;
            }
            Some(PendingFont {
                resource_name,
                bytes: used.loaded.bytes,
                glyphs: used.glyphs,
            })
        })
        .collect();
    Ok(DrawnText {
        operations,
        fonts: pending,
    })
}

pub fn authored_text_bounds(
    page: &Page,
    text: &TextBox,
    base_dir: &Path,
) -> Result<(f64, f64), RenderError> {
    let shown = chosen_text(text)?;
    if shown.is_empty() {
        return Ok((0.0, 0.0));
    }
    let parsed = markup::parse(shown, text)?;
    let fonts = load_fonts(page, text, base_dir, &parsed)?;
    let advances = build_advances(&fonts, &parsed.items, &text.id)?;
    let units = page.page_size.units;
    let (content_width, _) = layout_size(text);
    let lines = layout(
        &parsed.items,
        &parsed.end_metrics,
        &advances,
        to_points(content_width, units),
        units,
        &text.id,
    )?;
    Ok(bounds_of(&lines, units))
}

fn fit_font_sizes(
    page: &Page,
    text: &TextBox,
    parsed: &mut markup::Markup,
    advances: &BTreeMap<String, Advances>,
) -> Result<Vec<LaidLine>, RenderError> {
    let units = page.page_size.units;
    let (content_width, content_height) = layout_size(text);
    if content_width <= 0.0 || content_height <= 0.0 {
        return Err(RenderError {
            context: text.id.clone(),
            message: "content width or height is 0".to_string(),
        });
    }
    let content_width_pt = to_points(content_width, units);
    let mut lines = layout(
        &parsed.items,
        &parsed.end_metrics,
        advances,
        content_width_pt,
        units,
        &text.id,
    )?;
    if lines.is_empty() {
        return Ok(lines);
    }
    // A taller run can sit in the middle of a line, so the first height misses it.
    // Shrinking can move that run to the end of a line and make the block taller.
    // Scale again from the new bounds while the line breaks keep changing.
    let mut previous = line_texts(&lines);
    for _ in 0..32 {
        let scale = fit_scale(&lines, content_width, content_height, units);
        if scale >= 1.0 {
            break;
        }
        scale_markup(parsed, scale);
        lines = layout(
            &parsed.items,
            &parsed.end_metrics,
            advances,
            content_width_pt,
            units,
            &text.id,
        )?;
        let next = line_texts(&lines);
        if next == previous {
            break;
        }
        previous = next;
    }
    Ok(lines)
}

fn line_texts(lines: &[LaidLine]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>()
        })
        .collect()
}

fn bounds_of(lines: &[LaidLine], units: Units) -> (f64, f64) {
    if lines.is_empty() {
        return (0.0, 0.0);
    }
    let width_pt = lines
        .iter()
        .map(|line| line.width_pt)
        .fold(0.0f32, f32::max);
    let width = f64::from(width_pt) / f64::from(to_points(1.0, units));
    (width, block_height_of(lines))
}

fn fit_scale(lines: &[LaidLine], content_width: f64, content_height: f64, units: Units) -> f64 {
    let text_width_pt = lines
        .iter()
        .map(|line| line.width_pt)
        .fold(0.0f32, f32::max);
    let text_height_pt = to_points(block_height_of(lines), units);
    let content_width_pt = to_points(content_width, units);
    let content_height_pt = to_points(content_height, units);
    if text_width_pt <= content_width_pt + 0.01 && text_height_pt <= content_height_pt + 0.01 {
        return 1.0;
    }
    let width_scale = if text_width_pt > 0.0 {
        f64::from(content_width_pt / text_width_pt)
    } else {
        1.0
    };
    let height_scale = if text_height_pt > 0.0 {
        f64::from(content_height_pt / text_height_pt)
    } else {
        1.0
    };
    width_scale.min(height_scale).min(1.0)
}

fn scale_markup(parsed: &mut markup::Markup, scale: f64) {
    for item in &mut parsed.items {
        match item {
            MarkupItem::Run(run) => {
                run.font_size *= scale;
                run.leading *= scale;
                run.line_height *= scale;
            }
            MarkupItem::Break(metrics) => scale_metrics(metrics, scale),
        }
    }
    scale_metrics(&mut parsed.end_metrics, scale);
}

fn scale_metrics(metrics: &mut LineMetrics, scale: f64) {
    metrics.font_size *= scale;
    metrics.leading *= scale;
    metrics.line_height *= scale;
}

fn load_fonts(
    page: &Page,
    text: &TextBox,
    base_dir: &Path,
    parsed: &markup::Markup,
) -> Result<BTreeMap<String, UsedFont>, RenderError> {
    let mut keys = Vec::new();
    for item in &parsed.items {
        match item {
            MarkupItem::Run(run) => push_key(&mut keys, &run.font_key),
            MarkupItem::Break(metrics) => push_key(&mut keys, &metrics.font_key),
        }
    }
    push_key(&mut keys, &parsed.end_metrics.font_key);

    let mut fonts = BTreeMap::new();
    for key in keys {
        let resource = page.fonts.get(&key).ok_or_else(|| RenderError {
            context: text.id.clone(),
            message: format!("unknown font {key}"),
        })?;
        let relative = resource.source_path.as_deref().ok_or_else(|| RenderError {
            context: text.id.clone(),
            message: "source_path is missing".to_string(),
        })?;
        let loaded = fonts::load_font(&base_dir.join(relative), &text.id)?;
        fonts.insert(
            key,
            UsedFont {
                loaded,
                resource_name: None,
                glyphs: Vec::new(),
                seen_glyphs: HashSet::new(),
            },
        );
    }
    Ok(fonts)
}

fn push_key(keys: &mut Vec<String>, key: &str) {
    if !keys.iter().any(|existing| existing == key) {
        keys.push(key.to_string());
    }
}

struct Advances {
    units_per_em: u16,
    glyphs: HashMap<char, (u16, u16)>,
}

fn build_advances(
    fonts: &BTreeMap<String, UsedFont>,
    items: &[MarkupItem],
    id: &str,
) -> Result<BTreeMap<String, Advances>, RenderError> {
    let mut needed: BTreeMap<String, Vec<char>> = BTreeMap::new();
    for item in items {
        if let MarkupItem::Run(run) = item {
            let chars = needed.entry(run.font_key.clone()).or_default();
            chars.push(' ');
            chars.extend(run.text.chars());
        }
    }
    let mut advances = BTreeMap::new();
    for (key, used) in fonts {
        let face = Face::parse(&used.loaded.bytes, 0).map_err(|err| RenderError {
            context: id.to_string(),
            message: format!("font file is not a valid TTF: {err}"),
        })?;
        let mut glyphs = HashMap::new();
        let mut missing = Vec::new();
        if let Some(chars) = needed.get(key) {
            for ch in chars {
                if glyphs.contains_key(ch) || missing.contains(ch) {
                    continue;
                }
                if let Some(glyph) = face.glyph_index(*ch) {
                    let advance = face.glyph_hor_advance(glyph).unwrap_or(0);
                    glyphs.insert(*ch, (glyph.0, advance));
                } else {
                    missing.push(*ch);
                }
            }
        }
        if !missing.is_empty() {
            return Err(fonts::uncovered(id, &missing));
        }
        advances.insert(
            key.clone(),
            Advances {
                units_per_em: used.loaded.units_per_em,
                glyphs,
            },
        );
    }
    Ok(advances)
}

fn layout(
    items: &[MarkupItem],
    end_metrics: &LineMetrics,
    advances: &BTreeMap<String, Advances>,
    content_width_pt: f32,
    units: Units,
    id: &str,
) -> Result<Vec<LaidLine>, RenderError> {
    let mut lines = Vec::new();
    let mut words = Vec::new();
    for item in items {
        match item {
            MarkupItem::Run(run) => {
                for piece in run.text.split(' ').filter(|word| !word.is_empty()) {
                    let word = word_from(run, piece, advances, units, id)?;
                    if words.is_empty()
                        || words_width(&words)
                            + words.last().expect("word").space_pt
                            + word.width_pt
                            <= content_width_pt + 0.01
                    {
                        words.push(word);
                        continue;
                    }
                    let ended = words.last().expect("line has a word").clone();
                    lines.push(finish_line(
                        std::mem::take(&mut words),
                        metrics_of_word(&ended),
                    ));
                    words.push(word);
                }
            }
            MarkupItem::Break(metrics) => {
                lines.push(finish_line(std::mem::take(&mut words), metrics.clone()));
            }
        }
    }
    if !words.is_empty() {
        lines.push(finish_line(words, end_metrics.clone()));
    }
    Ok(lines)
}

fn word_from(
    run: &StyledRun,
    text: &str,
    advances: &BTreeMap<String, Advances>,
    units: Units,
    id: &str,
) -> Result<Word, RenderError> {
    Ok(Word {
        width_pt: measure_advances(advances, &run.font_key, text, run.font_size, units, id)?,
        space_pt: measure_advances(advances, &run.font_key, " ", run.font_size, units, id)?,
        text: text.to_string(),
        font_key: run.font_key.clone(),
        font_size: run.font_size,
        leading: run.leading,
        line_height: run.line_height,
        underline: run.underline,
    })
}

fn metrics_of_word(word: &Word) -> LineMetrics {
    LineMetrics {
        font_key: word.font_key.clone(),
        font_size: word.font_size,
        leading: word.leading,
        line_height: word.line_height,
    }
}

fn finish_line(words: Vec<Word>, metrics: LineMetrics) -> LaidLine {
    let (segments, width_pt) = segments_of(&words);
    LaidLine {
        segments,
        width_pt,
        line_height: metrics.line_height,
        leading: metrics.leading,
        font_key: metrics.font_key,
        font_size: metrics.font_size,
    }
}

fn words_width(words: &[Word]) -> f32 {
    let mut total = 0.0;
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            total += words[index - 1].space_pt;
        }
        total += word.width_pt;
    }
    total
}

fn segments_of(words: &[Word]) -> (Vec<Segment>, f32) {
    let mut segments: Vec<Segment> = Vec::new();
    let mut pending_space = 0.0;
    for word in words {
        if let Some(last) = segments.last_mut() {
            if last.font_key == word.font_key
                && last.font_size == word.font_size
                && last.underline == word.underline
            {
                last.text.push(' ');
                last.text.push_str(&word.text);
                last.width_pt += pending_space + word.width_pt;
                pending_space = word.space_pt;
                continue;
            }
            last.text.push(' ');
            last.width_pt += pending_space;
        }
        segments.push(Segment {
            text: word.text.clone(),
            font_key: word.font_key.clone(),
            font_size: word.font_size,
            underline: word.underline,
            width_pt: word.width_pt,
        });
        pending_space = word.space_pt;
    }
    let width_pt = segments.iter().map(|segment| segment.width_pt).sum();
    (segments, width_pt)
}

fn measure_advances(
    advances: &BTreeMap<String, Advances>,
    key: &str,
    text: &str,
    font_size: f64,
    units: Units,
    id: &str,
) -> Result<f32, RenderError> {
    let font = advances.get(key).ok_or_else(|| RenderError {
        context: id.to_string(),
        message: format!("unknown font {key}"),
    })?;
    let size_pt = to_points(font_size, units);
    let units_per_em = f32::from(font.units_per_em);
    let mut missing = Vec::new();
    let mut width = 0.0f32;
    for ch in text.chars() {
        match font.glyphs.get(&ch) {
            Some((_, advance)) => width += f32::from(*advance) * size_pt / units_per_em,
            None => {
                if !missing.contains(&ch) {
                    missing.push(ch);
                }
            }
        }
    }
    if !missing.is_empty() {
        return Err(fonts::uncovered(id, &missing));
    }
    Ok(width)
}

fn paint_lines(
    page: &Page,
    text: &TextBox,
    fonts: &mut BTreeMap<String, UsedFont>,
    advances: &BTreeMap<String, Advances>,
    font_index: &mut usize,
    lines: &[LaidLine],
) -> Result<Vec<Operation>, RenderError> {
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let units = page.page_size.units;
    let (content_width, content_height) = layout_size(text);
    let content_x = 0.0;
    let content_y = 0.0;
    let block_height = block_height_of(lines);
    let (first_key, first_size) = line_face(&lines[0]);
    let first_ascent = ascent(fonts, first_key, first_size);
    let block_top = match text.vertical {
        VerticalAlign::Top => content_y,
        VerticalAlign::Middle => content_y + (content_height - block_height) / 2.0,
        VerticalAlign::Bottom => {
            let (last_key, last_size) = line_face(lines.last().expect("line"));
            let descent = descent(fonts, last_key, last_size);
            let steps: f64 = lines[..lines.len() - 1]
                .iter()
                .map(|line| line.line_height + line.leading)
                .sum();
            let first_baseline = content_y + content_height - descent - steps;
            first_baseline - first_ascent
        }
    };

    let scale = f64::from(to_points(1.0, units));
    let (clip_x, clip_y) = pdf_point(page, text.pos_x, text.pos_y + text.height);
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "re",
            vec![
                Object::Real(clip_x),
                Object::Real(clip_y),
                Object::Real(to_points(text.width, units)),
                Object::Real(to_points(text.height, units)),
            ],
        ),
        Operation::new("W", vec![]),
        Operation::new("n", vec![]),
    ];

    let mut baseline = block_top + first_ascent;
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            let previous = &lines[index - 1];
            baseline += previous.line_height + previous.leading;
        }
        let mut cursor = match text.horizontal {
            HorizontalAlign::Left => content_x,
            HorizontalAlign::Center => {
                content_x + (content_width - f64::from(line.width_pt) / scale) / 2.0
            }
            HorizontalAlign::Right => content_x + content_width - f64::from(line.width_pt) / scale,
        };
        for segment in &line.segments {
            let encoded = encode_segment(fonts, advances, font_index, segment, &text.id)?;
            let width_page = f64::from(segment.width_pt) / scale;
            let (pdf_x, pdf_y) = place(page, text, cursor, baseline);
            let resource_name = fonts
                .get(&segment.font_key)
                .and_then(|used| used.resource_name.clone())
                .expect("font resource");
            operations.push(Operation::new("BT", vec![]));
            operations.push(Operation::new(
                "Tf",
                vec![
                    Object::Name(resource_name.into_bytes()),
                    Object::Real(to_points(segment.font_size, units)),
                ],
            ));
            operations.push(Operation::new("Tm", text_matrix(text.rotation, pdf_x, pdf_y)));
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(encoded, StringFormat::Hexadecimal)],
            ));
            operations.push(Operation::new("ET", vec![]));
            if segment.underline && segment.width_pt > 0.0 {
                let font = &fonts.get(&segment.font_key).expect("font").loaded;
                let underline_y = baseline + underline_drop(font, segment.font_size);
                let (x0, y0) = place(page, text, cursor, underline_y);
                let (x1, y1) = place(page, text, cursor + width_page, underline_y);
                operations.push(Operation::new("w", vec![Object::Real(0.5)]));
                operations.push(Operation::new("m", vec![Object::Real(x0), Object::Real(y0)]));
                operations.push(Operation::new("l", vec![Object::Real(x1), Object::Real(y1)]));
                operations.push(Operation::new("S", vec![]));
            }
            cursor += width_page;
        }
    }
    operations.push(Operation::new("Q", vec![]));
    Ok(operations)
}

fn layout_size(text: &TextBox) -> (f64, f64) {
    let width = text.width - text.padding.left - text.padding.right;
    let height = text.height - text.padding.top - text.padding.bottom;
    if text.rotation == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

fn place(page: &Page, text: &TextBox, local_x: f64, local_y: f64) -> (f32, f32) {
    let (document_x, document_y) = if text.rotation == 90 {
        let along = text.height - text.padding.top - text.padding.bottom;
        (
            text.pos_x + text.padding.left + local_y,
            text.pos_y + text.padding.top + along - local_x,
        )
    } else {
        (
            text.pos_x + text.padding.left + local_x,
            text.pos_y + text.padding.top + local_y,
        )
    };
    pdf_point(page, document_x, document_y)
}

fn text_matrix(rotation: i32, pdf_x: f32, pdf_y: f32) -> Vec<Object> {
    let (a, b, c, d) = if rotation == 90 {
        // Advance up the page. Glyphs stand to the left of that baseline.
        (0.0, 1.0, -1.0, 0.0)
    } else {
        (1.0, 0.0, 0.0, 1.0)
    };
    vec![
        Object::Real(a),
        Object::Real(b),
        Object::Real(c),
        Object::Real(d),
        Object::Real(pdf_x),
        Object::Real(pdf_y),
    ]
}

fn encode_segment(
    fonts: &mut BTreeMap<String, UsedFont>,
    advances: &BTreeMap<String, Advances>,
    font_index: &mut usize,
    segment: &Segment,
    id: &str,
) -> Result<Vec<u8>, RenderError> {
    let used = fonts
        .get_mut(&segment.font_key)
        .ok_or_else(|| RenderError {
            context: id.to_string(),
            message: format!("unknown font {}", segment.font_key),
        })?;
    if used.resource_name.is_none() {
        *font_index += 1;
        used.resource_name = Some(format!("F{font_index}"));
    }
    let font = advances_for(advances, &segment.font_key, id)?;
    let run = fonts::encode_glyphs(&font.glyphs, &segment.text, id)?;
    for (gid, unicode) in run.glyphs {
        if used.seen_glyphs.insert(gid) {
            used.glyphs.push((gid, unicode));
        }
    }
    Ok(run.encoded)
}

fn advances_for<'a>(
    advances: &'a BTreeMap<String, Advances>,
    key: &str,
    id: &str,
) -> Result<&'a Advances, RenderError> {
    advances.get(key).ok_or_else(|| RenderError {
        context: id.to_string(),
        message: format!("unknown font {key}"),
    })
}

fn line_face(line: &LaidLine) -> (&str, f64) {
    if let Some(segment) = line.segments.first() {
        (segment.font_key.as_str(), segment.font_size)
    } else {
        (line.font_key.as_str(), line.font_size)
    }
}

fn block_height_of(lines: &[LaidLine]) -> f64 {
    let last = lines.last().expect("line");
    let mut height = last.line_height;
    for line in &lines[..lines.len() - 1] {
        height += line.line_height + line.leading;
    }
    height
}

fn ascent(fonts: &BTreeMap<String, UsedFont>, key: &str, font_size: f64) -> f64 {
    fonts::ascent_in_page_units(&fonts.get(key).expect("font").loaded, font_size)
}

fn descent(fonts: &BTreeMap<String, UsedFont>, key: &str, font_size: f64) -> f64 {
    fonts::descent_in_page_units(&fonts.get(key).expect("font").loaded, font_size)
}

fn underline_drop(font: &LoadedFont, font_size: f64) -> f64 {
    let face = Face::parse(&font.bytes, 0).ok();
    let position =
        face.and_then(|parsed| parsed.underline_metrics().map(|metrics| metrics.position));
    match position {
        Some(position) if position < 0 => {
            font_size * f64::from(-position) / f64::from(font.units_per_em)
        }
        _ => font_size * 0.1,
    }
}

fn chosen_text(text: &TextBox) -> Result<&str, RenderError> {
    if let Some(content) = &text.text.content {
        return Ok(content);
    }
    if let Some(preentered) = &text.text.preentered {
        return Ok(preentered);
    }
    Err(RenderError {
        context: text.id.clone(),
        message: "preentered and content are both missing".to_string(),
    })
}
