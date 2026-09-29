use lopdf::Dictionary;

/// Utility functions for PDF processing
pub struct Utils;

impl Utils {
    /// Create a BMP file from raw image data
    pub fn create_bmp_from_raw(
        data: &[u8],
        dict: &Dictionary,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // Get image properties
        let width = if let Ok(w) = dict.get(b"Width") {
            if let lopdf::Object::Integer(w) = w {
                *w as u32
            } else {
                return Err("Invalid width".into());
            }
        } else {
            return Err("No width".into());
        };

        let height = if let Ok(h) = dict.get(b"Height") {
            if let lopdf::Object::Integer(h) = h {
                *h as u32
            } else {
                return Err("Invalid height".into());
            }
        } else {
            return Err("No height".into());
        };

        let bits_per_component = if let Ok(bpc) = dict.get(b"BitsPerComponent") {
            if let lopdf::Object::Integer(bpc) = bpc {
                *bpc as u16
            } else {
                return Err("Invalid bits per component".into());
            }
        } else {
            return Err("No bits per component".into());
        };

        // Determine color space
        let is_rgb = Self::is_rgb_color_space(dict);

        let bytes_per_pixel = if is_rgb { 3 } else { 1 };
        let row_size = ((width * bytes_per_pixel * bits_per_component as u32 / 8 + 3) / 4) * 4; // BMP row padding
        let image_size = row_size * height;
        let file_size = 54 + image_size;

        let mut bmp = Vec::new();

        // BMP Header
        bmp.extend_from_slice(b"BM"); // Signature
        bmp.extend_from_slice(&file_size.to_le_bytes()); // File size
        bmp.extend_from_slice(&[0, 0, 0, 0]); // Reserved
        bmp.extend_from_slice(&54u32.to_le_bytes()); // Data offset

        // DIB Header
        bmp.extend_from_slice(&40u32.to_le_bytes()); // Header size
        bmp.extend_from_slice(&width.to_le_bytes()); // Width
        bmp.extend_from_slice(&height.to_le_bytes()); // Height
        bmp.extend_from_slice(&1u16.to_le_bytes()); // Planes
        bmp.extend_from_slice(&(bits_per_component * if is_rgb { 3 } else { 1 }).to_le_bytes()); // Bits per pixel
        bmp.extend_from_slice(&0u32.to_le_bytes()); // Compression
        bmp.extend_from_slice(&image_size.to_le_bytes()); // Image size
        bmp.extend_from_slice(&0u32.to_le_bytes()); // X pixels per meter
        bmp.extend_from_slice(&0u32.to_le_bytes()); // Y pixels per meter
        bmp.extend_from_slice(&0u32.to_le_bytes()); // Colors in color table
        bmp.extend_from_slice(&0u32.to_le_bytes()); // Important color count

        // Image data (flip vertically for BMP format)
        for row in (0..height).rev() {
            let start = (row * width * bytes_per_pixel) as usize;
            let end = start + (width * bytes_per_pixel) as usize;
            if end <= data.len() {
                bmp.extend_from_slice(&data[start..end]);
                // Add row padding
                let padding = row_size - (width * bytes_per_pixel);
                for _ in 0..padding {
                    bmp.push(0);
                }
            }
        }

        Ok(bmp)
    }

    /// Determine if the color space is RGB
    fn is_rgb_color_space(dict: &Dictionary) -> bool {
        if let Ok(cs) = dict.get(b"ColorSpace") {
            match cs {
                lopdf::Object::Name(cs_bytes) => {
                    let cs_str = String::from_utf8_lossy(cs_bytes);
                    cs_str == "DeviceRGB" || cs_str == "/DeviceRGB"
                }
                lopdf::Object::Array(cs_array) => {
                    // Check for ICCBased or other RGB color spaces
                    if !cs_array.is_empty() {
                        if let lopdf::Object::Name(first_cs) = &cs_array[0] {
                            let cs_str = String::from_utf8_lossy(first_cs);
                            cs_str == "ICCBased" || cs_str == "/ICCBased"
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }
}
