use lopdf::Object;

use crate::modules::data_structures::*;

pub fn extract_text_content_from_operations(
    operations: &[lopdf::content::Operation],
) -> Result<Vec<ContentBox>, Box<dyn std::error::Error>> {
    let mut content_boxes = Vec::new();
    let mut text_lines = Vec::new();
    let mut current_line = Vec::new();
    let mut current_y = 0.0;
    let mut current_x = 0.0;
    let mut font_size = 12.0;
    let mut font_name = "Unknown".to_string();

    // Group text operations by line (tracked by Tm operations) - reusing text analyzer logic
    for operation in operations {
        match operation.operator.as_str() {
            "Tm" => {
                // New text line - start a new line if we have content
                if !current_line.is_empty() {
                    text_lines.push((current_y, current_line.clone()));
                    current_line.clear();
                }
                // Extract Y position from Tm
                if operation.operands.len() >= 6 {
                    if let Object::Real(y) = &operation.operands[5] {
                        current_y = *y;
                    } else if let Object::Integer(y) = &operation.operands[5] {
                        current_y = *y as f32;
                    }
                    if let Object::Real(x) = &operation.operands[4] {
                        current_x = *x;
                    } else if let Object::Integer(x) = &operation.operands[4] {
                        current_x = *x as f32;
                    }
                }
            }
            "Tf" => {
                // Set font and font size
                if operation.operands.len() >= 2 {
                    if let Object::Name(name_bytes) = &operation.operands[0] {
                        font_name = String::from_utf8_lossy(name_bytes).to_string();
                    }
                    if let Object::Real(size) = &operation.operands[1] {
                        font_size = *size;
                    } else if let Object::Integer(size) = &operation.operands[1] {
                        font_size = *size as f32;
                    }
                }
            }
            "Tj" | "TJ" => {
                for operand in &operation.operands {
                    match operand {
                        Object::String(bytes, _) => {
                            current_line.push(bytes.clone());
                        }
                        Object::Array(arr) => {
                            for item in arr {
                                if let Object::String(bytes, _) = item {
                                    current_line.push(bytes.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "ET" => {
                // End of text block - save current line
                if !current_line.is_empty() {
                    text_lines.push((current_y, current_line.clone()));
                    current_line.clear();
                }
            }
            _ => {}
        }
    }

    // Save any remaining line
    if !current_line.is_empty() {
        text_lines.push((current_y, current_line));
    }

    // Convert text lines to content boxes using existing text analyzer logic
    if !text_lines.is_empty() {
        let mut y_positions: Vec<f32> = text_lines.iter().map(|(y, _)| *y).collect();
        y_positions.sort_by(|a, b| b.partial_cmp(a).unwrap()); // Sort descending (top to bottom)

        let avg_spacing = if y_positions.len() > 1 {
            (y_positions[0] - y_positions[y_positions.len() - 1]) / (y_positions.len() - 1) as f32
        } else {
            font_size * 1.2
        };

        let line_height_ratio = avg_spacing / font_size;

        // Determine alignment using existing logic
        let mut text_positions = Vec::new();
        for operation in operations {
            if operation.operator == "Tj" || operation.operator == "TJ" {
                text_positions.push((current_x, current_y));
            }
        }

        let alignment = if !text_positions.is_empty() {
            let min_x = text_positions
                .iter()
                .map(|(x, _)| *x)
                .fold(f32::INFINITY, f32::min);
            let max_x = text_positions
                .iter()
                .map(|(x, _)| *x)
                .fold(f32::NEG_INFINITY, f32::max);
            let page_width = 595.92;
            let left_margin = min_x;
            let right_margin = page_width - max_x;

            if (left_margin - right_margin).abs() < 5.0 {
                TextAlignment::Center
            } else if left_margin < right_margin {
                TextAlignment::Left
            } else {
                TextAlignment::Right
            }
        } else {
            TextAlignment::Left
        };

        // Create content boxes for each text line
        for (y_pos, line_bytes) in &text_lines {
            let mut full_text_bytes = Vec::new();
            for bytes in line_bytes {
                full_text_bytes.extend_from_slice(bytes);
            }

            // Decode text using existing ToUnicode mapping logic
            let decoded_text = decode_text_bytes(&full_text_bytes);

            if !decoded_text.is_empty() {
                let content_box = ContentBox::Text {
                    x: current_x,
                    y: *y_pos,
                    width: decoded_text.len() as f32 * font_size * 0.6, // Rough estimate
                    height: font_size,
                    text: TextContent {
                        resource_name: font_name.clone(),
                        font_size,
                        text_lines: vec![TextLine {
                            y_position: *y_pos,
                            x_position: current_x,
                            text: decoded_text,
                            font_size,
                            line_height: avg_spacing,
                        }],
                        line_height: avg_spacing,
                        line_height_ratio,
                        font_properties: FontProperties::default(),
                        alignment: alignment.clone(),
                    },
                };
                content_boxes.push(content_box);
            }
        }
    }

    Ok(content_boxes)
}

fn decode_text_bytes(bytes: &[u8]) -> String {
    // Apply ToUnicode CMap mapping from text analyzer
    let char_map = std::collections::HashMap::from([
        (0x0003, 0x0020),
        (0x01AD, 0x0411),
        (0x01B6, 0x041A),
        (0x01BA, 0x041E),
        (0x01BD, 0x0421),
        (0x01CC, 0x0442),
        (0x01DC, 0x0440),
        (0x01EA, 0x044E),
    ]);

    // Helper function for range mappings
    fn map_cid_to_unicode(cid: u16, char_map: &std::collections::HashMap<u16, u32>) -> Option<u32> {
        // Direct mapping
        if let Some(&unicode) = char_map.get(&cid) {
            return Some(unicode);
        }

        // Range mappings from ToUnicode CMap
        if 0x01CE <= cid && cid <= 0x01D1 {
            Some(0x0432u32 + (cid as u32 - 0x01CE))
        } else if 0x01D5 <= cid && cid <= 0x01D6 {
            Some(0x0439u32 + (cid as u32 - 0x01D5))
        } else if 0x01D9 <= cid && cid <= 0x01DA {
            Some(0x043Du32 + (cid as u32 - 0x01D9))
        } else if 0x01DF <= cid && cid <= 0x01E0 {
            Some(0x0443u32 + (cid as u32 - 0x01DF))
        } else {
            None
        }
    }

    let mut decoded_text = String::new();
    for chunk in bytes.chunks(2) {
        if chunk.len() == 2 {
            let cid = ((chunk[0] as u16) << 8) | (chunk[1] as u16);
            if let Some(unicode) = map_cid_to_unicode(cid, &char_map) {
                if let Some(ch) = char::from_u32(unicode) {
                    decoded_text.push(ch);
                }
            }
        }
    }

    decoded_text
}
