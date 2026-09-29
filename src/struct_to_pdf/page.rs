use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

#[derive(Debug)]
pub struct RenderError {
    pub context: String,
    pub message: String,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.context, self.message)
    }
}

impl std::error::Error for RenderError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceFormat {
    Yaml,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Units {
    Mm,
    Points,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bleeds {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
    pub units: Units,
    pub bleeds: Bleeds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontResource {
    pub name: String,
    pub font_family: Option<String>,
    pub weight: Option<String>,
    pub italic: Option<bool>,
    pub embedded: Option<bool>,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageResource {
    pub name: String,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub file_format: Option<String>,
    pub extracted_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub page_size: PageSize,
    pub fonts: BTreeMap<String, FontResource>,
    pub images: BTreeMap<String, ImageResource>,
    pub contents: Vec<ContentEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContentEntry {
    Text(TextBox),
    Image(ImagePlacement),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HorizontalAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerticalAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontStyle {
    pub weight: String,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextBody {
    pub font: String,
    pub font_size: f64,
    pub leading: f64,
    pub line_height: f64,
    pub font_style: FontStyle,
    pub preentered: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    pub id: String,
    pub pos_x: f64,
    pub pos_y: f64,
    pub width: f64,
    pub height: f64,
    pub auto_scale: bool,
    pub horizontal: HorizontalAlign,
    pub vertical: VerticalAlign,
    pub padding: Padding,
    pub text: TextBody,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImagePlacement {
    pub id: String,
    pub pos_x: f64,
    pub pos_y: f64,
    pub width: f64,
    pub height: f64,
    pub resource_name: String,
}

pub fn load_page(path: &Path) -> Result<Page, RenderError> {
    let source = fs::read_to_string(path).map_err(|err| RenderError {
        context: "page".to_string(),
        message: err.to_string(),
    })?;
    let format = match path.extension().and_then(|ext| ext.to_str()) {
        Some("yaml") | Some("yml") => SourceFormat::Yaml,
        Some("json") => SourceFormat::Json,
        _ => {
            return Err(RenderError {
                context: "page".to_string(),
                message: "page file must be .yaml, .yml, or .json".to_string(),
            })
        }
    };
    parse_page(&source, format)
}

pub fn parse_page(source: &str, format: SourceFormat) -> Result<Page, RenderError> {
    let value = match format {
        SourceFormat::Yaml => {
            let yaml: serde_yaml::Value =
                serde_yaml::from_str(source).map_err(|err| RenderError {
                    context: "page".to_string(),
                    message: err.to_string(),
                })?;
            serde_json::to_value(yaml).map_err(|err| RenderError {
                context: "page".to_string(),
                message: err.to_string(),
            })?
        }
        SourceFormat::Json => serde_json::from_str(source).map_err(|err| RenderError {
            context: "page".to_string(),
            message: err.to_string(),
        })?,
    };
    parse_value(&value)
}

fn parse_value(value: &Value) -> Result<Page, RenderError> {
    let root = value.as_object().ok_or_else(|| RenderError {
        context: "page".to_string(),
        message: "page must be an object".to_string(),
    })?;
    if root.contains_key("pages") {
        return Err(RenderError {
            context: "pages".to_string(),
            message: "v0 takes one page, not a document".to_string(),
        });
    }
    if root.contains_key("metadata") {
        return Err(RenderError {
            context: "metadata".to_string(),
            message: "v0 takes one page, not a document".to_string(),
        });
    }

    let page_size = parse_page_size(root.get("page_size"))?;
    let fonts = parse_fonts(root.get("resources"))?;
    let images = parse_images(root.get("resources"))?;
    let contents = parse_contents(root.get("contents"))?;

    Ok(Page {
        page_size,
        fonts,
        images,
        contents,
    })
}

fn parse_page_size(value: Option<&Value>) -> Result<PageSize, RenderError> {
    let obj = object(value, "page_size")?;
    let width = required_finite(obj, "width", "page_size.width", false)?;
    let height = required_finite(obj, "height", "page_size.height", false)?;
    let units = match obj.get("units").and_then(Value::as_str) {
        Some("mm") => Units::Mm,
        Some("points") => Units::Points,
        Some(_) => {
            return Err(RenderError {
                context: "units".to_string(),
                message: "units must be mm or points".to_string(),
            })
        }
        None => {
            return Err(RenderError {
                context: "units".to_string(),
                message: "units is missing".to_string(),
            })
        }
    };
    let bleeds = parse_bleeds(obj.get("bleeds"))?;
    Ok(PageSize {
        width,
        height,
        units,
        bleeds,
    })
}

fn parse_bleeds(value: Option<&Value>) -> Result<Bleeds, RenderError> {
    let obj = match value {
        Some(Value::Object(obj)) => obj,
        Some(_) => {
            return Err(RenderError {
                context: "bleeds".to_string(),
                message: "bleeds must be an object".to_string(),
            })
        }
        None => {
            return Err(RenderError {
                context: "bleeds".to_string(),
                message: "bleeds is missing".to_string(),
            })
        }
    };
    Ok(Bleeds {
        top: bleed_side(obj, "top")?,
        right: bleed_side(obj, "right")?,
        bottom: bleed_side(obj, "bottom")?,
        left: bleed_side(obj, "left")?,
    })
}

fn bleed_side(obj: &Map<String, Value>, side: &str) -> Result<f64, RenderError> {
    let context = format!("bleeds.{side}");
    match obj.get(side) {
        None => Err(RenderError {
            context,
            message: "bleed side is missing".to_string(),
        }),
        Some(value) => {
            let number = as_f64(value).ok_or_else(|| RenderError {
                context: context.clone(),
                message: "bleed side must be a number".to_string(),
            })?;
            if !number.is_finite() {
                return Err(RenderError {
                    context,
                    message: "bleed side must be finite".to_string(),
                });
            }
            if number < 0.0 {
                return Err(RenderError {
                    context,
                    message: "bleed side must be >= 0".to_string(),
                });
            }
            Ok(number)
        }
    }
}

fn parse_fonts(resources: Option<&Value>) -> Result<BTreeMap<String, FontResource>, RenderError> {
    let resources = object(resources, "resources")?;
    let fonts = object(resources.get("fonts"), "resources.fonts")?;
    let mut out = BTreeMap::new();
    for (key, value) in fonts {
        let obj = value.as_object().ok_or_else(|| RenderError {
            context: format!("resources.fonts.{key}"),
            message: "font must be an object".to_string(),
        })?;
        out.insert(
            key.clone(),
            FontResource {
                name: obj
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(key)
                    .to_string(),
                font_family: optional_string(obj, "font_family"),
                weight: optional_string(obj, "weight"),
                italic: obj.get("italic").and_then(Value::as_bool),
                embedded: obj.get("embedded").and_then(Value::as_bool),
                source_path: optional_string(obj, "source_path"),
            },
        );
    }
    Ok(out)
}

fn parse_images(resources: Option<&Value>) -> Result<BTreeMap<String, ImageResource>, RenderError> {
    let resources = object(resources, "resources")?;
    let images = object(resources.get("images"), "resources.images")?;
    let mut out = BTreeMap::new();
    for (key, value) in images {
        let obj = value.as_object().ok_or_else(|| RenderError {
            context: format!("resources.images.{key}"),
            message: "image must be an object".to_string(),
        })?;
        out.insert(
            key.clone(),
            ImageResource {
                name: obj
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(key)
                    .to_string(),
                width: optional_number(obj, "width"),
                height: optional_number(obj, "height"),
                file_format: optional_string(obj, "file_format"),
                extracted_path: optional_string(obj, "extracted_path"),
            },
        );
    }
    Ok(out)
}

fn optional_number(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    match obj.get(key)? {
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

fn parse_contents(value: Option<&Value>) -> Result<Vec<ContentEntry>, RenderError> {
    let items = match value {
        Some(Value::Array(items)) => items,
        Some(_) => {
            return Err(RenderError {
                context: "contents".to_string(),
                message: "contents must be an array".to_string(),
            })
        }
        None => {
            return Err(RenderError {
                context: "contents".to_string(),
                message: "contents is missing".to_string(),
            })
        }
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| parse_content(index, item))
        .collect()
}

fn parse_content(index: usize, value: &Value) -> Result<ContentEntry, RenderError> {
    let obj = value.as_object().ok_or_else(|| RenderError {
        context: format!("contents[{index}]"),
        message: "content entry must be an object".to_string(),
    })?;
    let has_text = obj.get("text").map(|v| !v.is_null()).unwrap_or(false);
    let has_image = obj.get("image").map(|v| !v.is_null()).unwrap_or(false);
    let id = optional_string(obj, "id").unwrap_or_default();

    if has_text && has_image {
        return Err(RenderError {
            context: id_context(index, &id),
            message: "content entry has both text and image".to_string(),
        });
    }
    if !has_text && !has_image {
        return Err(RenderError {
            context: if id.is_empty() {
                format!("contents[{index}]")
            } else {
                id
            },
            message: "content entry has neither text nor image".to_string(),
        });
    }
    if id.is_empty() {
        return Err(RenderError {
            context: format!("contents[{index}].id"),
            message: "id is missing".to_string(),
        });
    }

    let pos_x = required_finite(obj, "posX", &format!("id={id} posX"), true)?;
    let pos_y = required_finite(obj, "posY", &format!("id={id} posY"), true)?;
    let width = required_finite(obj, "width", &format!("id={id}"), false)?;
    let height = required_finite(obj, "height", &format!("id={id}"), false)?;

    if has_text {
        let text_obj = object(obj.get("text"), &format!("id={id} text"))?;
        Ok(ContentEntry::Text(TextBox {
            id: id.clone(),
            pos_x,
            pos_y,
            width,
            height,
            auto_scale: parse_auto_scale(obj, &id)?,
            horizontal: parse_horizontal(obj, &format!("id={}", text_obj_id_from(obj)))?,
            vertical: parse_vertical(obj, &format!("id={}", text_obj_id_from(obj)))?,
            padding: parse_padding(obj, width, height, &text_obj_id_from(obj))?,
            text: parse_text(text_obj, &text_obj_id_from(obj))?,
        }))
    } else {
        let image_obj = object(obj.get("image"), &format!("id={id} image"))?;
        let resource_name =
            optional_string(image_obj, "resource_name").ok_or_else(|| RenderError {
                context: id.clone(),
                message: "resource_name is missing".to_string(),
            })?;
        Ok(ContentEntry::Image(ImagePlacement {
            id,
            pos_x,
            pos_y,
            width,
            height,
            resource_name,
        }))
    }
}

fn text_obj_id_from(obj: &Map<String, Value>) -> String {
    optional_string(obj, "id").unwrap_or_default()
}

fn parse_auto_scale(obj: &Map<String, Value>, id: &str) -> Result<bool, RenderError> {
    match obj.get("auto_scale") {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(RenderError {
            context: id.to_string(),
            message: "auto_scale must be a boolean".to_string(),
        }),
    }
}

fn parse_horizontal(
    obj: &Map<String, Value>,
    context: &str,
) -> Result<HorizontalAlign, RenderError> {
    let alignment = object(obj.get("alignment"), &format!("{context} alignment"))?;
    match alignment.get("horizontal").and_then(Value::as_str) {
        Some("left") => Ok(HorizontalAlign::Left),
        Some("center") => Ok(HorizontalAlign::Center),
        Some("right") => Ok(HorizontalAlign::Right),
        Some(_) => Err(RenderError {
            context: context.to_string(),
            message: "horizontal must be left, center, or right".to_string(),
        }),
        None => Err(RenderError {
            context: context.to_string(),
            message: "horizontal is missing".to_string(),
        }),
    }
}

fn parse_vertical(obj: &Map<String, Value>, context: &str) -> Result<VerticalAlign, RenderError> {
    let alignment = object(obj.get("alignment"), &format!("{context} alignment"))?;
    match alignment.get("vertical").and_then(Value::as_str) {
        Some("top") => Ok(VerticalAlign::Top),
        Some("middle") => Ok(VerticalAlign::Middle),
        Some("bottom") => Ok(VerticalAlign::Bottom),
        Some(_) => Err(RenderError {
            context: context.to_string(),
            message: "vertical must be top, middle, or bottom".to_string(),
        }),
        None => Err(RenderError {
            context: context.to_string(),
            message: "vertical is missing".to_string(),
        }),
    }
}

fn parse_padding(
    obj: &Map<String, Value>,
    width: f64,
    height: f64,
    id: &str,
) -> Result<Padding, RenderError> {
    let padding = object(obj.get("padding"), &format!("id={id} padding"))?;
    let top = padding_side(padding, "top", id)?;
    let right = padding_side(padding, "right", id)?;
    let bottom = padding_side(padding, "bottom", id)?;
    let left = padding_side(padding, "left", id)?;
    if left + right > width || top + bottom > height {
        return Err(RenderError {
            context: format!("id={id}"),
            message: "padding is wider or taller than the frame".to_string(),
        });
    }
    Ok(Padding {
        top,
        right,
        bottom,
        left,
    })
}

fn padding_side(obj: &Map<String, Value>, side: &str, id: &str) -> Result<f64, RenderError> {
    match obj.get(side) {
        None => Err(RenderError {
            context: format!("id={id} padding.{side}"),
            message: "padding side is missing".to_string(),
        }),
        Some(value) => {
            let number = as_f64(value).ok_or_else(|| RenderError {
                context: format!("id={id} padding.{side}"),
                message: "padding side must be a number".to_string(),
            })?;
            if number < 0.0 || !number.is_finite() {
                return Err(RenderError {
                    context: format!("id={id}"),
                    message: "padding side must be >= 0".to_string(),
                });
            }
            Ok(number)
        }
    }
}

fn parse_text(obj: &Map<String, Value>, id: &str) -> Result<TextBody, RenderError> {
    let font = optional_string(obj, "font").ok_or_else(|| RenderError {
        context: format!("id={id} font"),
        message: "font is missing".to_string(),
    })?;
    let font_size = required_finite(obj, "font_size", &format!("id={id} font_size"), true)?;
    let leading = match obj.get("leading") {
        None => {
            return Err(RenderError {
                context: format!("id={id} leading"),
                message: "leading is missing".to_string(),
            })
        }
        Some(value) => as_f64(value).ok_or_else(|| RenderError {
            context: format!("id={id} leading"),
            message: "leading must be a number".to_string(),
        })?,
    };
    if !leading.is_finite() {
        return Err(RenderError {
            context: format!("id={id} leading"),
            message: "leading must be finite".to_string(),
        });
    }
    let line_height = match obj.get("line_height") {
        None => {
            return Err(RenderError {
                context: format!("id={id} line_height"),
                message: "line_height is missing".to_string(),
            })
        }
        Some(value) => as_f64(value).ok_or_else(|| RenderError {
            context: format!("id={id} line_height"),
            message: "line_height must be a number".to_string(),
        })?,
    };
    if !line_height.is_finite() {
        return Err(RenderError {
            context: format!("id={id} line_height"),
            message: "line_height must be finite".to_string(),
        });
    }
    Ok(TextBody {
        font,
        font_size,
        leading,
        line_height,
        font_style: parse_font_style(obj.get("font_style")),
        preentered: optional_text(obj.get("preentered")),
        content: optional_text(obj.get("content")),
    })
}

fn parse_font_style(value: Option<&Value>) -> FontStyle {
    let Some(Value::Object(obj)) = value else {
        return FontStyle {
            weight: "normal".to_string(),
            italic: false,
            underline: false,
        };
    };
    FontStyle {
        weight: optional_string(obj, "weight").unwrap_or_else(|| "normal".to_string()),
        italic: obj.get("italic").and_then(Value::as_bool).unwrap_or(false),
        underline: obj
            .get("underline")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn optional_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

fn id_context(index: usize, id: &str) -> String {
    if id.is_empty() {
        format!("contents[{index}].id")
    } else {
        format!("id={id}")
    }
}

fn object<'a>(
    value: Option<&'a Value>,
    context: &str,
) -> Result<&'a Map<String, Value>, RenderError> {
    match value {
        Some(Value::Object(obj)) => Ok(obj),
        _ => Err(RenderError {
            context: context.to_string(),
            message: "must be an object".to_string(),
        }),
    }
}

fn optional_string(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_string)
}

fn required_finite(
    obj: &Map<String, Value>,
    key: &str,
    context: &str,
    allow_negative: bool,
) -> Result<f64, RenderError> {
    let number = match obj.get(key) {
        None => {
            return Err(RenderError {
                context: context.to_string(),
                message: format!("{key} is missing"),
            })
        }
        Some(value) => as_f64(value).ok_or_else(|| RenderError {
            context: context.to_string(),
            message: format!("{key} must be a number"),
        })?,
    };
    if !number.is_finite() || (!allow_negative && number < 0.0) {
        return Err(RenderError {
            context: context.to_string(),
            message: format!("{key} must be finite and >= 0"),
        });
    }
    Ok(number)
}

fn as_f64(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_i64().map(|n| n as f64))
}
