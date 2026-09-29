use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Complete PDF analysis result
#[derive(Debug, Serialize, Deserialize)]
pub struct PdfAnalysis {
    pub pages: Vec<PageAnalysis>,
    pub metadata: PdfMetadata,
}

/// PDF metadata information
#[derive(Debug, Serialize, Deserialize)]
pub struct PdfMetadata {
    pub page_count: usize,
    pub title: Option<String>,
    pub author: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub modification_date: Option<String>,
}

/// Analysis of a single page
#[derive(Debug, Serialize, Deserialize)]
pub struct PageAnalysis {
    pub page_number: usize,
    pub page_id: String,
    pub page_size: PageSize,
    pub resources: PageResources,
    pub contents: Vec<ContentBox>,
}

/// Page size information
#[derive(Debug, Serialize, Deserialize)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
    pub units: String,
    pub media_box: Option<[f32; 4]>,
}

/// Resources available on a page
#[derive(Debug, Serialize, Deserialize)]
pub struct PageResources {
    pub images: HashMap<String, ImageResource>,
    pub fonts: HashMap<String, FontResource>,
}

/// Image resource information
#[derive(Debug, Serialize, Deserialize)]
pub struct ImageResource {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u16,
    pub color_space: String,
    pub filter: String,
    pub file_format: String,
    pub extracted_path: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub description: String,
}

/// Font resource information
#[derive(Debug, Serialize, Deserialize)]
pub struct FontResource {
    pub name: String,
    pub base_font: Option<String>,
    pub font_type: Option<String>,
    pub encoding: Option<String>,
    pub to_unicode: Option<String>,
    pub extracted_path: Option<String>,
    pub description: String,
}

/// Content box with positioning and content information
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentBox {
    Text {
        #[serde(rename = "posX")]
        x: f32,
        #[serde(rename = "posY")]
        y: f32,
        width: f32,
        height: f32,
        text: TextContent,
    },
    Image {
        #[serde(rename = "posX")]
        x: f32,
        #[serde(rename = "posY")]
        y_raw: f32,
        #[serde(rename = "posY_Pdf")]
        y_pdf: f32,
        width: f32,
        height: f32,
        image: ImageContent,
    },
}

/// Text content information
#[derive(Debug, Serialize, Deserialize)]
pub struct TextContent {
    pub resource_name: String,
    pub font_size: f32,
    pub text_lines: Vec<TextLine>,
    pub line_height: f32,
    pub line_height_ratio: f32,
    pub font_properties: FontProperties,
    pub alignment: TextAlignment,
}

/// Image content information
#[derive(Debug, Serialize, Deserialize)]
pub struct ImageContent {
    pub resource_name: String,
    pub transformation_matrix: [f32; 6],
}

/// Individual text line with positioning
#[derive(Debug, Serialize, Deserialize)]
pub struct TextLine {
    pub y_position: f32,
    pub x_position: f32,
    pub text: String,
    pub font_size: f32,
    pub line_height: f32,
}

/// Font properties
#[derive(Debug, Serialize, Deserialize)]
pub struct FontProperties {
    pub is_italic: bool,
    pub is_bold: bool,
    pub is_underlined: bool,
    pub font_family: Option<String>,
}

/// Text alignment information
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
    Justified,
}

impl Default for FontProperties {
    fn default() -> Self {
        Self {
            is_italic: false,
            is_bold: false,
            is_underlined: false,
            font_family: None,
        }
    }
}

/// Text state for tracking current text formatting and position
#[derive(Debug, Clone)]
pub struct TextState {
    pub x: f32,
    pub y: f32,
    pub font_name: String,
    pub font_size: f32,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            font_name: "Unknown".to_string(),
            font_size: 12.0,
        }
    }
}
