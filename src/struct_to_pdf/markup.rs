use crate::struct_to_pdf::{RenderError, TextBox};

#[derive(Clone, Debug)]
pub struct StyledRun {
    pub text: String,
    pub font_key: String,
    pub font_size: f64,
    pub leading: f64,
    pub line_height: f64,
    pub underline: bool,
}

#[derive(Clone, Debug)]
pub struct LineMetrics {
    pub font_key: String,
    pub font_size: f64,
    pub leading: f64,
    pub line_height: f64,
}

#[derive(Clone, Debug)]
pub enum MarkupItem {
    Run(StyledRun),
    Break(LineMetrics),
}

pub struct Markup {
    pub items: Vec<MarkupItem>,
    pub end_metrics: LineMetrics,
}

#[derive(Clone)]
struct Style {
    font_key: String,
    font_size: f64,
    leading: f64,
    line_height: f64,
    weight: String,
    italic: bool,
    underline: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum FrameKind {
    Root,
    Bold,
    Italic,
    Underline,
    Span,
}

struct Frame {
    kind: FrameKind,
    style: Style,
}

pub fn parse(input: &str, text: &TextBox) -> Result<Markup, RenderError> {
    let mut stack = vec![Frame {
        kind: FrameKind::Root,
        style: Style {
            font_key: text.text.font.clone(),
            font_size: text.text.font_size,
            leading: text.text.leading,
            line_height: text.text.line_height,
            weight: text.text.font_style.weight.clone(),
            italic: text.text.font_style.italic,
            underline: text.text.font_style.underline,
        },
    }];
    let mut items = Vec::new();
    let mut buffer = String::new();
    let mut rest = input;
    while !rest.is_empty() {
        if let Some(after_amp) = rest.strip_prefix('&') {
            let (ch, consumed) = decode_entity(after_amp, &text.id)?;
            buffer.push(ch);
            rest = &after_amp[consumed..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('\n') {
            flush_run(&mut buffer, &stack, &mut items);
            items.push(MarkupItem::Break(metrics(current(&stack))));
            rest = after;
            continue;
        }
        if let Some(after) = rest.strip_prefix('<') {
            flush_run(&mut buffer, &stack, &mut items);
            if let Some(after_open) = after.strip_prefix("leading>") {
                rest = take_leading(after_open, &mut stack, &text.id)?;
                continue;
            }
            let (tag, next) = split_tag(after, &text.id)?;
            apply_tag(tag, &mut stack, &mut items, &text.id)?;
            rest = next;
            continue;
        }
        let ch = rest.chars().next().expect("non-empty");
        buffer.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    flush_run(&mut buffer, &stack, &mut items);
    if stack.len() != 1 {
        return Err(error(&text.id, "unclosed tag"));
    }
    Ok(Markup {
        items,
        end_metrics: metrics(current(&stack)),
    })
}

fn take_leading<'a>(
    after_open: &'a str,
    stack: &mut [Frame],
    id: &str,
) -> Result<&'a str, RenderError> {
    let end = after_open
        .find("</leading>")
        .ok_or_else(|| error(id, "leading tag is missing its closer"))?;
    let body = after_open[..end].trim();
    let value = body
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or_else(|| error(id, "leading body is not a number"))?;
    current_mut(stack).leading = value;
    Ok(&after_open[end + "</leading>".len()..])
}

fn apply_tag(
    tag: &str,
    stack: &mut Vec<Frame>,
    items: &mut Vec<MarkupItem>,
    id: &str,
) -> Result<(), RenderError> {
    let tag = tag.trim();
    if let Some(closer) = tag.strip_prefix('/') {
        return apply_closer(closer.trim(), stack, id);
    }
    let (name, attrs) = split_name(tag);
    match name {
        "br" => {
            if !attrs.is_empty() {
                return Err(error(id, "unknown tag br"));
            }
            items.push(MarkupItem::Break(metrics(current(stack))));
            Ok(())
        }
        "b" | "bold" => {
            if !attrs.is_empty() {
                return Err(error(id, format!("unknown tag {name}")));
            }
            push_flag(stack, FrameKind::Bold, |style| {
                style.weight = "bold".to_string()
            });
            Ok(())
        }
        "i" => {
            if !attrs.is_empty() {
                return Err(error(id, "unknown tag i"));
            }
            push_flag(stack, FrameKind::Italic, |style| style.italic = true);
            Ok(())
        }
        "u" => {
            if !attrs.is_empty() {
                return Err(error(id, "unknown tag u"));
            }
            push_flag(stack, FrameKind::Underline, |style| style.underline = true);
            Ok(())
        }
        "span" => {
            let mut style = current(stack).clone();
            apply_span_attrs(attrs, &mut style, id)?;
            stack.push(Frame {
                kind: FrameKind::Span,
                style,
            });
            Ok(())
        }
        "leading" => Err(error(id, "leading body is not a number")),
        other => Err(error(id, format!("unknown tag {other}"))),
    }
}

fn apply_closer(closer: &str, stack: &mut Vec<Frame>, id: &str) -> Result<(), RenderError> {
    let (name, rest) = split_name(closer);
    if !rest.is_empty() {
        return Err(error(id, format!("unknown tag {name}")));
    }
    match name {
        "b" | "bold" => pop(stack, FrameKind::Bold, id),
        "i" => pop(stack, FrameKind::Italic, id),
        "u" => pop(stack, FrameKind::Underline, id),
        "span" => pop(stack, FrameKind::Span, id),
        other => Err(error(id, format!("unknown tag {other}"))),
    }
}

fn apply_span_attrs(attrs: &str, style: &mut Style, id: &str) -> Result<(), RenderError> {
    for attr in parse_attrs(attrs, id)? {
        match attr.name.as_str() {
            "font" => style.font_key = attr.value,
            "leading" => style.leading = number(&attr.value, id)?,
            "line-height" => style.line_height = number(&attr.value, id)?,
            "style" => apply_style_list(&attr.value, style, id)?,
            _ => return Err(error(id, "unknown tag span")),
        }
    }
    Ok(())
}

fn apply_style_list(value: &str, style: &mut Style, id: &str) -> Result<(), RenderError> {
    for part in value.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, raw) = part
            .split_once(':')
            .ok_or_else(|| error(id, format!("unknown style {part}")))?;
        let key = key.trim();
        let raw = raw.trim();
        match key {
            "font-size" => style.font_size = number(raw, id)?,
            "font-weight" => match raw {
                "normal" | "bold" => style.weight = raw.to_string(),
                _ => return Err(error(id, format!("unknown style {key}"))),
            },
            "font-style" => match raw {
                "normal" => style.italic = false,
                "italic" => style.italic = true,
                _ => return Err(error(id, format!("unknown style {key}"))),
            },
            "text-decoration" => match raw {
                "none" => style.underline = false,
                "underline" => style.underline = true,
                _ => return Err(error(id, format!("unknown style {key}"))),
            },
            _ => return Err(error(id, format!("unknown style {key}"))),
        }
    }
    Ok(())
}

struct Attr {
    name: String,
    value: String,
}

fn parse_attrs(input: &str, id: &str) -> Result<Vec<Attr>, RenderError> {
    let mut attrs = Vec::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        let name_end = rest
            .find(|ch: char| ch.is_whitespace() || ch == '=')
            .unwrap_or(rest.len());
        if name_end == 0 {
            return Err(error(id, "unknown tag span"));
        }
        let name = rest[..name_end].to_string();
        rest = rest[name_end..].trim_start();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let quoted = after
                .strip_prefix('"')
                .ok_or_else(|| error(id, "unknown tag span"))?;
            let end = quoted
                .find('"')
                .ok_or_else(|| error(id, "unknown tag span"))?;
            attrs.push(Attr {
                name,
                value: quoted[..end].to_string(),
            });
            rest = quoted[end + 1..].trim_start();
        } else {
            return Err(error(id, "unknown tag span"));
        }
    }
    Ok(attrs)
}

fn push_flag(stack: &mut Vec<Frame>, kind: FrameKind, update: impl FnOnce(&mut Style)) {
    let mut style = current(stack).clone();
    update(&mut style);
    stack.push(Frame { kind, style });
}

fn pop(stack: &mut Vec<Frame>, kind: FrameKind, id: &str) -> Result<(), RenderError> {
    match stack.last() {
        Some(frame) if frame.kind == kind => {
            stack.pop();
            Ok(())
        }
        _ => Err(error(id, "unclosed tag")),
    }
}

fn flush_run(buffer: &mut String, stack: &[Frame], items: &mut Vec<MarkupItem>) {
    if buffer.is_empty() {
        return;
    }
    let style = current(stack);
    items.push(MarkupItem::Run(StyledRun {
        text: std::mem::take(buffer),
        font_key: style.font_key.clone(),
        font_size: style.font_size,
        leading: style.leading,
        line_height: style.line_height,
        underline: style.underline,
    }));
}

fn current(stack: &[Frame]) -> &Style {
    &stack.last().expect("style stack").style
}

fn current_mut(stack: &mut [Frame]) -> &mut Style {
    &mut stack.last_mut().expect("style stack").style
}

fn metrics(style: &Style) -> LineMetrics {
    LineMetrics {
        font_key: style.font_key.clone(),
        font_size: style.font_size,
        leading: style.leading,
        line_height: style.line_height,
    }
}

fn decode_entity(after_amp: &str, id: &str) -> Result<(char, usize), RenderError> {
    if after_amp.starts_with("lt;") {
        return Ok(('<', "lt;".len()));
    }
    if after_amp.starts_with("gt;") {
        return Ok(('>', "gt;".len()));
    }
    if after_amp.starts_with("amp;") {
        return Ok(('&', "amp;".len()));
    }
    Err(error(id, "unknown entity"))
}

fn split_tag<'a>(after_lt: &'a str, id: &str) -> Result<(&'a str, &'a str), RenderError> {
    let end = after_lt
        .find('>')
        .ok_or_else(|| error(id, "unclosed tag"))?;
    Ok((&after_lt[..end], &after_lt[end + 1..]))
}

fn split_name(tag: &str) -> (&str, &str) {
    let tag = tag.trim();
    let end = tag.find(char::is_whitespace).unwrap_or(tag.len());
    (tag[..end].trim(), tag[end..].trim())
}

fn number(value: &str, id: &str) -> Result<f64, RenderError> {
    value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| error(id, "leading body is not a number"))
}

fn error(id: &str, message: impl Into<String>) -> RenderError {
    RenderError {
        context: id.to_string(),
        message: message.into(),
    }
}
