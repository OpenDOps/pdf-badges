use std::collections::HashMap;
use std::path::Path;

use crate::struct_to_pdf::{
    load_page, prepare_visitor, render as render_raster, render_visitor, Raster,
};

#[derive(Debug)]
pub struct PageFileError {
    pub message: String,
}

impl std::fmt::Display for PageFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub fn page_pdf(
    category_dir: &Path,
    values: &HashMap<String, String>,
    photos: &HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, PageFileError> {
    let path = page_path(category_dir)?;
    let page = load_page(&path).map_err(|error| PageFileError {
        message: error.message,
    })?;
    render_visitor(&page, values, photos, category_dir).map_err(page_error)
}

pub fn render(
    category_dir: &Path,
    values: &HashMap<String, String>,
    photos: &HashMap<String, Vec<u8>>,
    dpi: f64,
) -> Result<Raster, PageFileError> {
    let path = page_path(category_dir)?;
    let page = load_page(&path).map_err(page_error)?;
    let page = prepare_visitor(&page, values, photos, category_dir).map_err(page_error)?;
    render_raster(&page, dpi, category_dir).map_err(page_error)
}

/// The Zebra values from `base/printer_conf/{name}.json` that become `^XA` lines.
#[derive(Debug, Clone, PartialEq)]
pub struct ZebraSettings {
    pub speed: String,
    pub darkness: String,
    pub print_mode: String,
    pub label_shift: String,
    pub label_top: String,
    pub tear_off: String,
}

impl Default for ZebraSettings {
    fn default() -> Self {
        Self {
            speed: "10".to_string(),
            darkness: "0".to_string(),
            print_mode: "Cutter".to_string(),
            label_shift: "0".to_string(),
            label_top: "0".to_string(),
            tear_off: "0".to_string(),
        }
    }
}

impl ZebraSettings {
    fn lines(&self) -> String {
        let mode = match self.print_mode.as_str() {
            "Tear" => "T",
            "Peel" => "P",
            "Rewind" => "R",
            "Applicator" => "A",
            _ => "C",
        };
        format!(
            "^PR{}\n^MD{}\n^MM{mode}\n^LS{}\n^LT{}\n~TA{}",
            self.speed, self.darkness, self.label_shift, self.label_top, self.tear_off
        )
    }
}

pub fn page_zpl(
    category_dir: &Path,
    values: &HashMap<String, String>,
    photos: &HashMap<String, Vec<u8>>,
    dpi: f64,
    printer_width_in: f64,
    settings: &ZebraSettings,
) -> Result<Vec<u8>, PageFileError> {
    let raster = render(category_dir, values, photos, dpi)?;
    let payload = ticket_render::png_graphic(&raster.png).map_err(|error| PageFileError {
        message: error.to_string(),
    })?;
    let printer_px = (printer_width_in * dpi).round() as i64;
    Ok(wrap_zpl(&payload, raster.width, printer_px, settings))
}

pub fn wrap_zpl(
    payload: &[u8],
    width_px: u32,
    printer_px: i64,
    settings: &ZebraSettings,
) -> Vec<u8> {
    let x_shift = (printer_px - i64::from(width_px)) / 2;
    let mut out = format!("^XA\n{}\n", settings.lines()).into_bytes();
    out.extend_from_slice(payload);
    out.extend_from_slice(
        format!("^LH0,0\n^FO{x_shift},0^IMG:BADGE.GRF^FS\n^IDR:BADGE.GRF^FS\n^MCY\n^XZ\n")
            .as_bytes(),
    );
    out
}

fn page_error(error: crate::struct_to_pdf::RenderError) -> PageFileError {
    PageFileError {
        message: error.message,
    }
}

fn page_path(category_dir: &Path) -> Result<std::path::PathBuf, PageFileError> {
    for name in ["page.yaml", "page.yml", "page.json"] {
        let path = category_dir.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(PageFileError {
        message: format!(
            "missing page file {}",
            category_dir.join("page.yaml").display()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use lopdf::Document;
    use std::io::Cursor;

    mod page_file {
        use super::*;

        #[test]
        fn yaml_page_is_pdf() {
            let dir = temp_dir("yaml");
            write_page(&dir, "page.yaml", &text_page("{{name}}", true, ""));
            let bytes = page_pdf(&dir, &values(&[("name", "Ann")]), &HashMap::new()).unwrap();
            let doc = Document::load_mem(&bytes).unwrap();
            assert!(doc.extract_text(&[1]).unwrap().contains("Ann"));
        }

        #[test]
        fn if_st_hides_the_entry() {
            let dir = temp_dir("ifst");
            let page = format!(
                "{}\n{}",
                text_entry("hidden", "HIDDEN", "possiblecat"),
                text_entry("shown", "SHOWN", "")
            );
            write_page(&dir, "page.yaml", &wrap_page(&page));
            let hidden =
                page_pdf(&dir, &values(&[("possiblecat", "false")]), &HashMap::new()).unwrap();
            let hidden_text = Document::load_mem(&hidden)
                .unwrap()
                .extract_text(&[1])
                .unwrap();
            assert!(!hidden_text.contains("HIDDEN"), "{hidden_text}");
            assert!(hidden_text.contains("SHOWN"), "{hidden_text}");

            let shown = page_pdf(&dir, &values(&[("possiblecat", "1")]), &HashMap::new()).unwrap();
            let shown_text = Document::load_mem(&shown)
                .unwrap()
                .extract_text(&[1])
                .unwrap();
            assert!(shown_text.contains("HIDDEN"), "{shown_text}");
            assert!(shown_text.contains("SHOWN"), "{shown_text}");
        }

        #[test]
        fn yaml_barcode_is_drawn() {
            let dir = temp_dir("barcode");
            write_page(
                &dir,
                "page.yaml",
                &wrap_page(&format!(
                    "{}\n{}",
                    barcode_entry("ean", "ean13", "9891160081678", 10.0),
                    barcode_entry("qr", "qr", "https://kuprin.su/", 40.0)
                )),
            );
            let bytes = page_pdf(&dir, &HashMap::new(), &HashMap::new()).unwrap();
            let images = xobjects(&bytes);
            assert!(images.len() >= 2, "{}", images.len());
            let ean = &images[0];
            assert!(ean.len() >= 113, "{}", ean.len());
            assert!(ean[..9].iter().all(|sample| *sample == 255));
            assert!(ean[ean.len() - 9..].iter().all(|sample| *sample == 255));
            assert_eq!(ean[9], 0);

            let qr = image::GrayImage::from_raw(
                (images[1].len() as f64).sqrt() as u32,
                (images[1].len() as f64).sqrt() as u32,
                images[1].clone(),
            )
            .expect("square qr");
            let mut prepared = rqrr::PreparedImage::prepare(qr);
            let grids = prepared.detect_grids();
            let (_, content) = grids[0].decode().unwrap();
            assert_eq!(content, "https://kuprin.su/");
        }

        #[test]
        fn yaml_photo_is_placed() {
            let dir = temp_dir("photo");
            write_page(&dir, "page.yaml", &wrap_page(&photo_entry("portrait", "")));
            let red = red_png();
            let bytes = page_pdf(
                &dir,
                &HashMap::new(),
                &HashMap::from([("photo".to_string(), red)]),
            )
            .unwrap();
            let images = xobjects(&bytes);
            assert!(images[0].chunks(3).any(|pixel| pixel == [255, 0, 0]));

            let missing = page_pdf(&dir, &HashMap::new(), &HashMap::new()).unwrap();
            assert!(xobjects(&missing).is_empty());

            write_page(
                &dir,
                "page.yaml",
                &wrap_page(&photo_entry("portrait", "possiblecat")),
            );
            let hidden = page_pdf(
                &dir,
                &values(&[("possiblecat", "false")]),
                &HashMap::from([("photo".to_string(), red_png())]),
            )
            .unwrap();
            assert!(xobjects(&hidden).is_empty());
        }

        #[test]
        fn missing_page_file_is_not_pdf() {
            let dir = temp_dir("missing");
            std::fs::write(dir.join("layout.json"), b"{}").unwrap();
            let error = page_pdf(&dir, &HashMap::new(), &HashMap::new()).unwrap_err();
            assert!(error.message.contains("page.yaml"), "{}", error.message);
        }

        fn temp_dir(name: &str) -> std::path::PathBuf {
            let dir =
                std::env::temp_dir().join(format!("page-file-{}-{}", std::process::id(), name));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn write_page(dir: &std::path::Path, name: &str, body: &str) {
            std::fs::write(dir.join(name), body).unwrap();
        }

        fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        }

        fn font_path() -> String {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf")
                .display()
                .to_string()
        }

        fn wrap_page(contents: &str) -> String {
            format!(
                r#"
page_size:
  width: 200
  height: 100
  units: points
  bleeds: {{top: 0, right: 0, bottom: 0, left: 0}}
resources:
  images: {{}}
  fonts:
    font1:
      name: font1
      embedded: true
      source_path: "{font}"
contents:
{contents}
"#,
                font = font_path()
            )
        }

        fn text_page(body: &str, template: bool, if_st: &str) -> String {
            wrap_page(
                &text_entry("line", body, if_st)
                    .replace("template: false", &format!("template: {template}")),
            )
        }

        fn text_entry(id: &str, body: &str, if_st: &str) -> String {
            format!(
                r#"
  - id: {id}
    ifSt: "{if_st}"
    posX: 10
    posY: 10
    width: 160
    height: 30
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    template: false
    text:
      font: font1
      font_size: 12
      leading: 0
      line_height: 14
      preentered: "{body}"
      content: null
"#
            )
        }

        fn barcode_entry(id: &str, kind: &str, payload: &str, pos_y: f64) -> String {
            format!(
                r#"
  - id: {id}
    posX: 10
    posY: {pos_y}
    width: 80
    height: 24
    barcode:
      type: {kind}
      preentered: "{payload}"
      content: null
"#
            )
        }

        fn photo_entry(id: &str, if_st: &str) -> String {
            format!(
                r#"
  - id: {id}
    ifSt: "{if_st}"
    posX: 10
    posY: 10
    width: 40
    height: 40
    photo:
      field: photo
"#
            )
        }

        fn red_png() -> Vec<u8> {
            let image = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]));
            let mut bytes = Vec::new();
            DynamicImage::ImageRgba8(image)
                .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
                .unwrap();
            bytes
        }

        fn xobjects(bytes: &[u8]) -> Vec<Vec<u8>> {
            let doc = Document::load_mem(bytes).unwrap();
            let page_id = *doc.get_pages().get(&1).unwrap();
            let resources = doc.get_dictionary(page_id).unwrap().get(b"Resources").ok();
            let Some(resources) = resources else {
                return Vec::new();
            };
            let resources = resources.as_dict().unwrap();
            let Ok(xobjects) = resources.get(b"XObject") else {
                return Vec::new();
            };
            let xobjects = xobjects.as_dict().unwrap();
            let mut names: Vec<_> = xobjects.iter().map(|(name, _)| name.clone()).collect();
            names.sort();
            names
                .into_iter()
                .map(|name| {
                    let id = xobjects
                        .get(name.as_slice())
                        .unwrap()
                        .as_reference()
                        .unwrap();
                    let stream = doc.get_object(id).unwrap().as_stream().unwrap();
                    stream.content.clone()
                })
                .collect()
        }
    }

    mod raster_file {
        use super::*;
        use image::Rgb;

        #[test]
        fn yaml_page_is_png() {
            let dir = temp_dir("png");
            write_page(&dir, "page.yaml", &badge_page());
            let drawn = render(
                &dir,
                &values(&[("name", "Ann"), ("possiblecat", "1")]),
                &photos(),
                100.0,
            )
            .unwrap();
            let image = decode_png(&drawn.png);
            assert_eq!(image.dimensions(), (200, 100));
            let ink = barcode_ink(&image);
            let hidden = render(
                &dir,
                &values(&[("name", "Ann"), ("possiblecat", "false")]),
                &photos(),
                100.0,
            )
            .unwrap();
            let hidden_image = decode_png(&hidden.png);
            assert_eq!(hidden_image.get_pixel(ink.0, ink.1).0, [255, 255, 255, 255]);
            assert!(text_has_ink(&image), "Ann is not drawn");
            let out = genbadges();
            std::fs::write(out.join("Ann.png"), &drawn.png).unwrap();
            std::fs::write(out.join("Ann.bmp"), &drawn.bmp).unwrap();
        }

        #[test]
        fn yaml_page_is_bmp() {
            let dir = temp_dir("bmp");
            write_page(&dir, "page.yaml", &badge_page());
            let drawn = render(
                &dir,
                &values(&[("name", "Ann"), ("possiblecat", "1")]),
                &photos(),
                100.0,
            )
            .unwrap();
            let image = decode_bmp(&drawn.bmp);
            assert_eq!(image.dimensions(), (200, 100));
            assert!(
                photo_rect(&image).all(|pixel| pixel == Rgb([255, 0, 0])),
                "photo rectangle is not the supplied image"
            );
            let missing = render(
                &dir,
                &values(&[("name", "Ann"), ("possiblecat", "1")]),
                &HashMap::new(),
                100.0,
            )
            .unwrap();
            let blank = decode_bmp(&missing.bmp);
            assert!(photo_rect(&blank).all(|pixel| pixel == Rgb([255, 255, 255])));
            let out = genbadges();
            std::fs::write(out.join("Ann.bmp"), &drawn.bmp).unwrap();
        }

        pub(super) fn badge_page() -> String {
            let font = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf")
                .display()
                .to_string();
            format!(
                r#"
page_size:
  width: 144
  height: 72
  units: points
  bleeds: {{top: 0, right: 0, bottom: 0, left: 0}}
resources:
  images: {{}}
  fonts:
    font1:
      name: font1
      embedded: true
      source_path: "{font}"
contents:
  - id: code
    ifSt: "possiblecat"
    posX: 0
    posY: 0
    width: 72
    height: 36
    barcode:
      type: ean13
      preentered: "9891160081678"
      content: null
  - id: name
    posX: 0
    posY: 40
    width: 70
    height: 30
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    template: true
    text:
      font: font1
      font_size: 18
      leading: 0
      line_height: 22
      preentered: "{{{{name}}}}"
      content: null
  - id: portrait
    posX: 72
    posY: 0
    width: 72
    height: 72
    photo:
      field: photo
"#
            )
        }

        pub(super) fn photos() -> HashMap<String, Vec<u8>> {
            HashMap::from([("photo".to_string(), red_png())])
        }

        fn red_png() -> Vec<u8> {
            let image = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255]));
            let mut bytes = Vec::new();
            DynamicImage::ImageRgba8(image)
                .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
                .unwrap();
            bytes
        }

        pub(super) fn temp_dir(name: &str) -> std::path::PathBuf {
            let dir =
                std::env::temp_dir().join(format!("raster-file-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        pub(super) fn write_page(dir: &std::path::Path, name: &str, body: &str) {
            std::fs::write(dir.join(name), body).unwrap();
        }

        pub(super) fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        }

        fn genbadges() -> std::path::PathBuf {
            let dir =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("target/registration-server/genbadges");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        pub(super) fn decode_png(bytes: &[u8]) -> RgbaImage {
            image::load_from_memory(bytes).unwrap().to_rgba8()
        }

        fn decode_bmp(bytes: &[u8]) -> image::RgbImage {
            image::load_from_memory(bytes).unwrap().to_rgb8()
        }

        pub(super) fn barcode_ink(image: &RgbaImage) -> (u32, u32) {
            for y in 0..50 {
                for x in 0..100 {
                    if image.get_pixel(x, y).0 != [255, 255, 255, 255] {
                        return (x, y);
                    }
                }
            }
            panic!("barcode left its box on the background");
        }

        fn text_has_ink(image: &RgbaImage) -> bool {
            for y in 56..100 {
                for x in 0..97 {
                    if image.get_pixel(x, y).0 != [255, 255, 255, 255] {
                        return true;
                    }
                }
            }
            false
        }

        fn photo_rect(image: &image::RgbImage) -> impl Iterator<Item = Rgb<u8>> + '_ {
            (0..100).flat_map(move |y| (100..200).map(move |x| *image.get_pixel(x, y)))
        }
    }

    mod zpl_file {
        use super::raster_file::{
            badge_page, barcode_ink, decode_png, photos, temp_dir, values, write_page,
        };
        use super::*;

        #[test]
        fn yaml_page_is_zpl() {
            let dir = temp_dir("zpl");
            write_page(&dir, "page.yaml", &badge_page());
            let shown = values(&[("name", "Ann"), ("possiblecat", "1")]);
            let settings = ZebraSettings::default();
            let body = page_zpl(&dir, &shown, &photos(), 100.0, 4.1, &settings).unwrap();
            let text = std::str::from_utf8(&body).unwrap();
            assert!(text.starts_with("^XA\n"), "{text}");
            assert!(text.trim_end().ends_with("^XZ"), "{text}");
            for line in ["^PR10", "^MD0", "^MMC", "^LS0", "^LT0", "~TA0", "^FO105,0"] {
                assert!(
                    text.lines().any(|row| row.starts_with(line)),
                    "{line} in {text}"
                );
            }

            let raster = render(&dir, &shown, &photos(), 100.0).unwrap();
            let payload = ticket_render::png_graphic(&raster.png).unwrap();
            let payload = std::str::from_utf8(&payload).unwrap();
            assert!(payload.starts_with("~DG"));
            assert!(text.contains(payload), "the ~DG payload is not in the body");

            let (x, y) = barcode_ink(&decode_png(&raster.png));
            assert!(
                bit(payload, x, y),
                "the drawn barcode is not in the graphic"
            );
            let hidden = values(&[("name", "Ann"), ("possiblecat", "false")]);
            let hidden_body = page_zpl(&dir, &hidden, &photos(), 100.0, 4.1, &settings).unwrap();
            let hidden_text = std::str::from_utf8(&hidden_body).unwrap();
            let hidden_payload = &hidden_text[hidden_text.find("~DG").unwrap()..];
            assert!(
                !bit(hidden_payload, x, y),
                "a hidden barcode is in the graphic"
            );
        }

        fn bit(payload: &str, x: u32, y: u32) -> bool {
            let row = payload.lines().nth(1 + y as usize).unwrap();
            let nibble = row
                .chars()
                .nth(x as usize / 4)
                .unwrap()
                .to_digit(16)
                .unwrap();
            (nibble >> (3 - x % 4)) & 1 == 1
        }
    }
}
