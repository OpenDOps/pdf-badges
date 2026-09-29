use lopdf::Object;
use std::fs::{create_dir_all, File};
use std::io::{Read, Write};

use crate::modules::data_structures::*;

pub fn extract_image_resource(
    name: &str,
    stream: &lopdf::Stream,
    page_num: usize,
) -> Result<ImageResource, Box<dyn std::error::Error>> {
    // Debug: Print all dictionary keys for this XObject
    println!("XObject '{}' dictionary keys:", name);
    for (key, _value) in stream.dict.iter() {
        println!("  - {}", String::from_utf8_lossy(key));
    }

    // Check if there's a matrix defined in the XObject resource itself
    if let Ok(matrix_obj) = stream.dict.get(b"Matrix") {
        println!("XObject '{}' has embedded matrix: {:?}", name, matrix_obj);
    }
    let width = if let Ok(w) = stream.dict.get(b"Width") {
        if let Object::Integer(w) = w {
            *w as u32
        } else {
            0
        }
    } else {
        0
    };

    let height = if let Ok(h) = stream.dict.get(b"Height") {
        if let Object::Integer(h) = h {
            *h as u32
        } else {
            0
        }
    } else {
        0
    };

    let bits_per_component = if let Ok(bpc) = stream.dict.get(b"BitsPerComponent") {
        if let Object::Integer(bpc) = bpc {
            *bpc as u16
        } else {
            8
        }
    } else {
        8
    };

    let color_space = if let Ok(cs) = stream.dict.get(b"ColorSpace") {
        format!("{:?}", cs)
    } else {
        "Unknown".to_string()
    };

    let filter = if let Ok(f) = stream.dict.get(b"Filter") {
        format!("{:?}", f)
    } else {
        "None".to_string()
    };

    let file_format = determine_image_format(&stream.dict)?;

    // Extract image to file
    let extracted_path = extract_image_to_file(stream, page_num, name, &file_format)?;

    let file_size_bytes = if let Some(path) = &extracted_path {
        std::fs::metadata(path).ok().map(|m| m.len())
    } else {
        None
    };

    let description = format!(
        "Image {}x{} {} {}bit",
        width, height, color_space, bits_per_component
    );

    Ok(ImageResource {
        name: name.to_string(),
        width,
        height,
        bits_per_component,
        color_space,
        filter,
        file_format,
        extracted_path,
        file_size_bytes,
        description,
    })
}

pub fn extract_image_content_from_operations(
    operations: &[lopdf::content::Operation],
    viewport_matrix: &[f32; 6],
    resources: &PageResources,
    transformation_matrix: &mut [f32; 6],
) -> Result<Vec<ContentBox>, Box<dyn std::error::Error>> {
    let mut content_boxes = Vec::new();

    for operation in operations {
        match operation.operator.as_str() {
            "cm" => {
                // Checking if there are 6 operands
                if operation.operands.len() == 6 {
                    // Read new matrix from operands
                    let mut operation_matrix = [0.0; 6];
                    for (i, operand) in operation.operands.iter().enumerate() {
                        if let Object::Real(r) = operand {
                            operation_matrix[i] = *r;
                        } else if let Object::Integer(i_val) = operand {
                            operation_matrix[i] = *i_val as f32;
                        }
                    }

                    // Concatenate matrices: result = current * new
                    // PDF matrix [a,b,c,d,e,f] represents 3x3 matrix:
                    // [a b 0]
                    // [c d 0]
                    // [e f 1]
                    // Convert to 3x3 matrices, multiply, then convert back
                    // Concatenate matrices: result = transformation_matrix * operation_matrix
                    // Using MuPDF's fz_concat formula
                    println!(
                        "Matrix concatenation: transformation={:?}, operation={:?}",
                        transformation_matrix, operation_matrix
                    );

                    // Apply MuPDF's fz_concat formula: result = transformation * operation
                    // Note: MuPDF uses fz_concat(new, old) which means result = new * old
                    // But in our case, transformation is the "old" and operation is the "new"
                    // MuPDF's fz_concat formula: result = new * old (where new is operation_matrix, old is transformation_matrix)
                    let result = [
                        operation_matrix[0] * transformation_matrix[0]
                            + operation_matrix[1] * transformation_matrix[2], // a
                        operation_matrix[0] * transformation_matrix[1]
                            + operation_matrix[1] * transformation_matrix[3], // b
                        operation_matrix[2] * transformation_matrix[0]
                            + operation_matrix[3] * transformation_matrix[2], // c
                        operation_matrix[2] * transformation_matrix[1]
                            + operation_matrix[3] * transformation_matrix[3], // d
                        operation_matrix[4] * transformation_matrix[0]
                            + operation_matrix[5] * transformation_matrix[2]
                            + transformation_matrix[4], // e
                        operation_matrix[4] * transformation_matrix[1]
                            + operation_matrix[5] * transformation_matrix[3]
                            + transformation_matrix[5], // f
                    ];

                    // Update the transformation matrix
                    *transformation_matrix = result;

                    println!("Result matrix: {:?}", transformation_matrix);
                }
            }
            "Do" => {
                // Draw XObject (image) - create image content box using existing logic
                if !operation.operands.is_empty() {
                    if let Object::Name(name_bytes) = &operation.operands[0] {
                        let name = String::from_utf8_lossy(name_bytes);

                        println!(
                            "Do operation for image '{}' with matrix: {:?}",
                            name, transformation_matrix
                        );

                        // Check if this is an image resource
                        if resources.images.contains_key(&name.to_string()) {
                            let image_resource = &resources.images[&name.to_string()];

                            // Extract position and scale from transformation matrix
                            // PDF transformation matrix: [a b c d e f] where:
                            // a, d: scale factors for X and Y
                            // e, f: translation (position)
                            // b, c: skew/rotation (usually 0 for images)
                            let x = transformation_matrix[4]; // X translation
                            let y_raw = transformation_matrix[5]; // Y translation (raw)

                            // PDF uses bottom-left coordinate system (Y=0 at bottom)
                            // We need to invert Y coordinate to match top-left system
                            let page_height = viewport_matrix[5];
                            let y = page_height - y_raw;

                            // Let's examine all matrix values to understand the coordinate system
                            println!(
                                "Image '{}': full transformation matrix: {:?}",
                                name, transformation_matrix
                            );
                            println!(
                                "Image '{}': raw Y={:.3}, converted Y={:.3} (page_height={:.3})",
                                name, y_raw, y, page_height
                            );

                            // Get image width and height from the image resource
                            let image_width = image_resource.width as f32;
                            let image_height = image_resource.height as f32;
                            println!(
                                "Image '{}': width={} height={}",
                                name, image_width, image_height
                            );

                            let content_box = ContentBox::Image {
                                x,
                                y_raw,
                                y_pdf: y,
                                width: transformation_matrix[0].abs(),
                                height: transformation_matrix[3].abs(),
                                image: ImageContent {
                                    resource_name: name.to_string(),
                                    transformation_matrix: transformation_matrix.clone(),
                                },
                            };
                            content_boxes.push(content_box);
                        }
                    }
                }
                *transformation_matrix = viewport_matrix.clone();
                println!("Transformation matrix RESET: {:?}", transformation_matrix);
            }
            op => {
                // Trace all other operations
                println!(
                    "Unhandled operation: '{}' with {} operands",
                    op,
                    operation.operands.len()
                );
                if !operation.operands.is_empty() {
                    println!("  Operands: {:?}", operation.operands);
                }
            }
        }
    }

    Ok(content_boxes)
}

fn determine_image_format(dict: &lopdf::Dictionary) -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(filter) = dict.get(b"Filter") {
        match filter {
            Object::Name(filter_bytes) => {
                let filter_str = String::from_utf8_lossy(filter_bytes);
                match &*filter_str {
                    "DCTDecode" => Ok("jpg".to_string()),
                    "FlateDecode" => {
                        // For FlateDecode, try to determine the best format
                        if let Ok(color_space) = dict.get(b"ColorSpace") {
                            match color_space {
                                Object::Name(cs_bytes) => {
                                    let cs_str = String::from_utf8_lossy(cs_bytes);
                                    match &*cs_str {
                                        "DeviceRGB" | "/DeviceRGB" => Ok("bmp".to_string()),
                                        "DeviceGray" | "/DeviceGray" => Ok("bmp".to_string()),
                                        "DeviceCMYK" | "/DeviceCMYK" => Ok("bmp".to_string()),
                                        _ => Ok("bmp".to_string()), // Default to BMP for better compatibility
                                    }
                                }
                                Object::Array(cs_array) => {
                                    // ICCBased or other complex color spaces
                                    if !cs_array.is_empty() {
                                        if let Object::Name(first_cs) = &cs_array[0] {
                                            let cs_str = String::from_utf8_lossy(first_cs);
                                            match &*cs_str {
                                                "ICCBased" => Ok("bmp".to_string()), // Assume RGB for ICC-based
                                                _ => Ok("bmp".to_string()),
                                            }
                                        } else {
                                            Ok("bmp".to_string())
                                        }
                                    } else {
                                        Ok("bmp".to_string())
                                    }
                                }
                                _ => Ok("bmp".to_string()),
                            }
                        } else {
                            Ok("bmp".to_string())
                        }
                    }
                    "JPXDecode" => Ok("jp2".to_string()),
                    _ => Ok("bmp".to_string()), // Default to BMP for unknown filters
                }
            }
            _ => Ok("bmp".to_string()),
        }
    } else {
        Ok("bmp".to_string())
    }
}

fn extract_image_to_file(
    stream: &lopdf::Stream,
    page_num: usize,
    name: &str,
    format: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    create_dir_all("extracted")?;

    let filename = format!("extracted/page{}_image_{}.{}", page_num, name, format);

    // Decompress image data first
    let image_data = decompress_image_data(stream)?;

    let mut file = File::create(&filename)?;

    if format == "bmp" {
        // Create BMP file with proper header
        if let Ok(bmp_data) =
            crate::modules::utils::Utils::create_bmp_from_raw(&image_data, &stream.dict)
        {
            file.write_all(&bmp_data)?;
        } else {
            // Fallback to raw data
            file.write_all(&image_data)?;
        }
    } else {
        file.write_all(&image_data)?;
    }

    Ok(Some(filename))
}

fn decompress_image_data(stream: &lopdf::Stream) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if let Ok(filter) = stream.dict.get(b"Filter") {
        match filter {
            Object::Name(filter_bytes) => {
                let filter_str = String::from_utf8_lossy(filter_bytes);
                match &*filter_str {
                    "FlateDecode" => {
                        // Decompress the data
                        let mut decoder = flate2::read::ZlibDecoder::new(&stream.content[..]);
                        let mut decompressed = Vec::new();
                        if decoder.read_to_end(&mut decompressed).is_ok() {
                            Ok(decompressed)
                        } else {
                            println!("    Failed to decompress, using raw data");
                            Ok(stream.content.clone())
                        }
                    }
                    _ => Ok(stream.content.clone()),
                }
            }
            _ => Ok(stream.content.clone()),
        }
    } else {
        Ok(stream.content.clone())
    }
}
