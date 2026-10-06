use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct LayoutError {
    message: String,
}

impl LayoutError {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LayoutError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub dots_per_point: f32,
    pub areas: Vec<Area>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub start_x: f64,
    pub start_y: f64,
    pub end_x: f64,
    pub end_y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextPart {
    pub text: String,
    pub is_break: bool,
    pub color: i32,
    pub font_size: f32,
    pub families: Vec<String>,
    pub weight: String,
    pub style: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AreaKind {
    Text {
        font_size: f32,
        families: Vec<String>,
        weight: String,
        style: String,
        parts: Vec<TextPart>,
    },
    Barcode {
        barcode_type: String,
    },
    Photo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Area {
    #[serde(rename = "box")]
    pub rect: Rect,
    pub alignment: Alignment,
    pub rotation: i32,
    pub color: i32,
    pub text: String,
    pub condition: String,
    #[serde(flatten)]
    pub kind: AreaKind,
}

pub fn load(bytes: &[u8]) -> Result<Layout, LayoutError> {
    serde_json::from_slice(bytes).map_err(|err| LayoutError {
        message: err.to_string(),
    })
}

pub fn store(layout: &Layout) -> Result<Vec<u8>, LayoutError> {
    serde_json::to_vec(layout).map_err(|err| LayoutError {
        message: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Layout {
        Layout {
            width: 288,
            height: 432,
            dots_per_point: 2.0,
            areas: vec![
                Area {
                    rect: Rect {
                        start_x: 1.0,
                        start_y: 2.0,
                        end_x: 30.0,
                        end_y: 14.0,
                    },
                    alignment: Alignment { x: 1, y: 0 },
                    rotation: 0,
                    color: 0,
                    text: "{{name}}".to_string(),
                    condition: "possiblecat".to_string(),
                    kind: AreaKind::Text {
                        font_size: 14.0,
                        families: vec!["Arial".to_string()],
                        weight: "normal".to_string(),
                        style: "normal".to_string(),
                        parts: vec![TextPart {
                            text: "{{name}}".to_string(),
                            is_break: false,
                            color: 0,
                            font_size: 14.0,
                            families: vec!["Times".to_string()],
                            weight: "bold".to_string(),
                            style: "italic".to_string(),
                        }],
                    },
                },
                Area {
                    rect: Rect {
                        start_x: 4.0,
                        start_y: 40.0,
                        end_x: 50.0,
                        end_y: 55.0,
                    },
                    alignment: Alignment { x: 0, y: 1 },
                    rotation: 1,
                    color: 0,
                    text: "{{code}}".to_string(),
                    condition: String::new(),
                    kind: AreaKind::Barcode {
                        barcode_type: "ean13".to_string(),
                    },
                },
                Area {
                    rect: Rect {
                        start_x: 60.0,
                        start_y: 8.0,
                        end_x: 80.0,
                        end_y: 32.0,
                    },
                    alignment: Alignment { x: 0, y: 0 },
                    rotation: 0,
                    color: 0,
                    text: String::new(),
                    condition: "photo".to_string(),
                    kind: AreaKind::Photo,
                },
            ],
        }
    }

    #[test]
    fn layout_roundtrip() {
        let layout = sample();
        let bytes = store(&layout).unwrap();
        let loaded = load(&bytes).unwrap();
        let again = load(&store(&loaded).unwrap()).unwrap();

        assert_eq!(again, layout);
        assert_eq!(again.width, 288);
        assert_eq!(again.height, 432);
        assert_eq!(again.dots_per_point, 2.0);
        assert_eq!(again.areas[0].rect.start_x, 1.0);
        assert_eq!(again.areas[0].rect.end_y, 14.0);
        assert_eq!(again.areas[0].condition, "possiblecat");
        match &again.areas[0].kind {
            AreaKind::Text { parts, .. } => {
                assert_eq!(parts[0].families, ["Times".to_string()]);
            }
            other => panic!("text area became {other:?}"),
        }
        match &again.areas[1].kind {
            AreaKind::Barcode { barcode_type } => assert_eq!(barcode_type, "ean13"),
            other => panic!("barcode area became {other:?}"),
        }
        assert!(matches!(again.areas[2].kind, AreaKind::Photo));
    }

    #[test]
    fn unknown_area_type_is_rejected() {
        let err = load(br#"{"width":1,"height":1,"dots_per_point":1.0,"areas":[{"box":{"start_x":0.0,"start_y":0.0,"end_x":1.0,"end_y":1.0},"alignment":{"x":0,"y":0},"rotation":0,"color":0,"text":"","condition":"","type":"stamp"}]}"#).unwrap_err();
        assert!(
            err.message().contains("stamp"),
            "message was {}",
            err.message()
        );
    }
}
