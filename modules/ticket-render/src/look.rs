use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::layout::Layout;
use crate::page::{err, PageError};
use crate::render_png;

pub fn write_badge_png(
    dir: &Path,
    layout: &Layout,
    values: &HashMap<String, String>,
    bg: &[u8],
) -> Result<PathBuf, PageError> {
    let dpi = lowered_dpi(layout);
    let png = render_png(layout, values, dpi, bg, None)?;
    let folder = dir.join("genbadges");
    fs::create_dir_all(&folder).map_err(|error| err(error.to_string()))?;
    let path = folder.join(file_name(values));
    fs::write(&path, png).map_err(|error| err(error.to_string()))?;
    Ok(path)
}

fn lowered_dpi(layout: &Layout) -> i32 {
    let mut dpi = 720;
    let mut divisor = 2;
    while long_side(layout, dpi) > 2400.0 {
        dpi = 720 / divisor;
        divisor += 1;
    }
    dpi.max(1)
}

fn long_side(layout: &Layout, dpi: i32) -> f32 {
    let width = inches(layout.width, layout.dots_per_point) * dpi as f32;
    let height = inches(layout.height, layout.dots_per_point) * dpi as f32;
    width.max(height)
}

fn inches(length: i32, dots_per_point: f32) -> f32 {
    (length as f32) / dots_per_point / 72.0
}

fn file_name(values: &HashMap<String, String>) -> String {
    let field = |key: &str| {
        values
            .get(key)
            .map(|value| clean_file_part(value))
            .unwrap_or_default()
    };
    format!(
        "{}_{}_{}_{}_{}.png",
        field("category"),
        field("name"),
        field("surname"),
        field("patronymic"),
        field("company")
    )
}

fn clean_file_part(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !reserved_in_file_name(*ch))
        .collect()
}

fn reserved_in_file_name(ch: char) -> bool {
    matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        || ch.is_control()
        || ch.is_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode_cfg;

    const VISITOR: &str = include_str!("../fixtures/visitor.txt");
    const CFG: &[u8] = include_bytes!("../fixtures/event/badge.cfg");
    const BG: &[u8] = include_bytes!("../fixtures/event/bg.png");

    fn visitor() -> HashMap<String, String> {
        VISITOR
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (key, value) = line.split_once(' ').unwrap();
                (key.to_string(), value.to_string())
            })
            .collect()
    }

    #[test]
    fn badge_png_is_written() {
        let layout = decode_cfg(CFG).unwrap();
        assert_eq!(layout.width, 3780);
        assert_eq!(layout.height, 5291);
        assert_eq!(layout.areas.len(), 4);
        let values = visitor();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ticket-render");
        let _ = fs::remove_dir_all(dir.join("genbadges"));
        let path = write_badge_png(&dir, &layout, &values, BG).unwrap();

        let file = format!("genbadges/{}", file_name(&values));
        let text = path.to_string_lossy().replace('\\', "/");
        assert!(text.ends_with(&file), "{text}");
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        let image = image::load_from_memory(&bytes).unwrap();
        let dpi = scala_dpi(&layout);
        let (width, height) = crate::page::page_px(&layout, dpi).unwrap();
        assert_eq!(image.width(), width);
        assert_eq!(image.height(), height);
        assert!(dir.join(&file).is_file());
    }

    #[test]
    fn file_name_drops_reserved_characters() {
        let values = HashMap::from([
            ("category".to_string(), "7".to_string()),
            ("name".to_string(), "Ann/Lee".to_string()),
            ("surname".to_string(), "O\"Brien".to_string()),
            ("patronymic".to_string(), "Ma:rie".to_string()),
            ("company".to_string(), "Irbis <HQ> | *?".to_string()),
        ]);
        assert_eq!(file_name(&values), "7_AnnLee_OBrien_Marie_IrbisHQ.png");
    }

    fn scala_dpi(layout: &Layout) -> i32 {
        let mut dpi = 720;
        let mut divisor = 2;
        while long_side(layout, dpi) > 2400.0 {
            dpi = 720 / divisor;
            divisor += 1;
        }
        dpi
    }
}
