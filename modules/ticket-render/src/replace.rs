use std::collections::HashMap;

use crate::layout::{Area, AreaKind};

pub fn replace(text: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = after[..close].trim().to_lowercase();
        if let Some(value) = values.get(&name) {
            out.push_str(value);
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

pub fn replace_areas(areas: &[Area], values: &HashMap<String, String>) -> Vec<Area> {
    areas
        .iter()
        .filter(|area| shown(&area.condition, values))
        .map(|area| replaced(area, values))
        .collect()
}

fn shown(condition: &str, values: &HashMap<String, String>) -> bool {
    if condition.is_empty() {
        return true;
    }
    match values.get(condition) {
        Some(value) if !value.is_empty() && value != "false" && value != "0" => true,
        _ => false,
    }
}

fn replaced(area: &Area, values: &HashMap<String, String>) -> Area {
    let mut area = area.clone();
    area.text = replace(&area.text, values);
    if let AreaKind::Text { parts, .. } = &mut area.kind {
        for part in parts {
            part.text = replace(&part.text, values);
        }
    }
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Alignment, AreaKind, Rect};

    fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn area(condition: &str) -> Area {
        Area {
            rect: Rect {
                start_x: 0.0,
                start_y: 0.0,
                end_x: 1.0,
                end_y: 1.0,
            },
            alignment: Alignment { x: 0, y: 0 },
            rotation: 0,
            color: 0,
            text: "Hi".to_string(),
            condition: condition.to_string(),
            kind: AreaKind::Photo,
        }
    }

    #[test]
    fn field_is_lowercased() {
        let map = values(&[("name", "Ann")]);
        assert_eq!(replace("{{Name}}", &map), "Ann");
    }

    #[test]
    fn missing_field_is_blank() {
        assert_eq!(replace("Hello {{name}}", &HashMap::new()), "Hello ");
    }

    #[test]
    fn ampersand_is_kept() {
        let map = values(&[("company", "A&B")]);
        assert_eq!(replace("{{company}}", &map), "A&B");
    }

    #[test]
    fn condition_hides_the_area() {
        let one = area("possiblecat");
        assert!(replace_areas(&[one.clone()], &HashMap::new()).is_empty());
        assert!(replace_areas(&[one.clone()], &values(&[("possiblecat", "")])).is_empty());
        assert!(replace_areas(&[one.clone()], &values(&[("possiblecat", "false")])).is_empty());
        assert!(replace_areas(&[one.clone()], &values(&[("possiblecat", "0")])).is_empty());
        assert_eq!(
            replace_areas(&[one.clone()], &values(&[("possiblecat", "1")])).len(),
            1
        );

        let always = area("");
        assert_eq!(replace_areas(&[always], &HashMap::new()).len(), 1);
    }
}
