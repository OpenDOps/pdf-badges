use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use image::RgbaImage;
use ticket_render::{AreaKind, Layout, PreparedFace};

use crate::struct_to_pdf::{
    index_template, load_page, prepare_visitor, render as render_raster, render_pixels,
    render_visitor, FaceSet, Page, Raster, TemplateIndex,
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
    let path = page_path(category_dir)?;
    let page = load_page(&path).map_err(page_error)?;
    let page = prepare_visitor(&page, values, photos, category_dir).map_err(page_error)?;
    let image =
        render_pixels(&page, dpi, category_dir, &FaceSet::from_page(&page)).map_err(page_error)?;
    let width = image.width();
    let payload = ticket_render::rgba_graphic(&image);
    let printer_px = (printer_width_in * dpi).round() as i64;
    Ok(wrap_zpl(&payload, width, printer_px, settings))
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

/// One open event. The key is the category directory name: the badge category, or that category plus 1000 for a certificate.
#[derive(Default)]
pub struct Badges {
    categories: BTreeMap<i64, CategoryEntry>,
}

pub struct CategoryEntry {
    pub layout: Option<Result<PreparedLayout, String>>,
    pub page: Option<Result<PreparedPage, String>>,
}

pub struct PreparedLayout {
    pub layout: Layout,
    pub background: RgbaImage,
    pub faces: Vec<PreparedFace>,
    scaled: HashMap<i32, RgbaImage>,
}

pub struct PreparedPage {
    pub page: Page,
    pub index: TemplateIndex,
    faces: FaceSet,
}

impl PreparedLayout {
    pub fn png(
        &self,
        values: &HashMap<String, String>,
        dpi: i32,
        photo: Option<&[u8]>,
    ) -> Result<Vec<u8>, PageFileError> {
        ticket_render::render_png_prepared(
            &self.layout,
            values,
            dpi,
            &self.background,
            photo,
            &self.faces,
            self.scaled.get(&dpi),
        )
        .map_err(|error| PageFileError {
            message: error.to_string(),
        })
    }

    pub fn graphic(
        &self,
        values: &HashMap<String, String>,
        dpi: i32,
    ) -> Result<Vec<u8>, PageFileError> {
        ticket_render::render_graphic_prepared(
            &self.layout,
            values,
            dpi,
            &self.background,
            &self.faces,
            self.scaled.get(&dpi),
        )
        .map_err(|error| PageFileError {
            message: error.to_string(),
        })
    }
}

impl PreparedPage {
    pub fn pdf(
        &self,
        values: &HashMap<String, String>,
        photos: &HashMap<String, Vec<u8>>,
    ) -> Result<Vec<u8>, PageFileError> {
        render_visitor(&self.page, values, photos, Path::new(".")).map_err(page_error)
    }

    fn pixels(
        &self,
        values: &HashMap<String, String>,
        photos: &HashMap<String, Vec<u8>>,
        dpi: f64,
    ) -> Result<RgbaImage, PageFileError> {
        let page =
            prepare_visitor(&self.page, values, photos, Path::new(".")).map_err(page_error)?;
        render_pixels(&page, dpi, Path::new("."), &self.faces).map_err(page_error)
    }

    pub fn png_bytes(
        &self,
        values: &HashMap<String, String>,
        photos: &HashMap<String, Vec<u8>>,
        dpi: f64,
    ) -> Result<Vec<u8>, PageFileError> {
        crate::struct_to_pdf::png_bytes(self.pixels(values, photos, dpi)?).map_err(page_error)
    }

    #[allow(dead_code)]
    pub fn bmp_bytes(
        &self,
        values: &HashMap<String, String>,
        photos: &HashMap<String, Vec<u8>>,
        dpi: f64,
    ) -> Result<Vec<u8>, PageFileError> {
        crate::struct_to_pdf::bmp_bytes(self.pixels(values, photos, dpi)?).map_err(page_error)
    }

    #[allow(dead_code)]
    pub fn graphic(
        &self,
        values: &HashMap<String, String>,
        photos: &HashMap<String, Vec<u8>>,
        dpi: f64,
    ) -> Result<(Vec<u8>, u32), PageFileError> {
        let image = self.pixels(values, photos, dpi)?;
        let width = image.width();
        Ok((ticket_render::rgba_graphic(&image), width))
    }
}

impl Badges {
    pub fn layout_size(&self, category: i64) -> Option<Result<(i32, i32), String>> {
        let entry = self.categories.get(&category)?;
        Some(match entry.layout.as_ref()? {
            Ok(layout) => Ok((layout.layout.width, layout.layout.height)),
            Err(message) => Err(message.clone()),
        })
    }

    pub fn layout_png(
        &self,
        category: i64,
        values: &HashMap<String, String>,
        dpi: i32,
    ) -> Option<Result<Vec<u8>, String>> {
        let entry = self.categories.get(&category)?;
        Some(match entry.layout.as_ref()? {
            Ok(layout) => layout.png(values, dpi, None).map_err(|error| error.message),
            Err(message) => Err(message.clone()),
        })
    }

    pub fn layout_graphic(
        &self,
        category: i64,
        values: &HashMap<String, String>,
        dpi: i32,
    ) -> Option<Result<Vec<u8>, String>> {
        let entry = self.categories.get(&category)?;
        Some(match entry.layout.as_ref()? {
            Ok(layout) => layout.graphic(values, dpi).map_err(|error| error.message),
            Err(message) => Err(message.clone()),
        })
    }

    pub fn page_pdf_bytes(
        &self,
        category: i64,
        values: &HashMap<String, String>,
        photos: &HashMap<String, Vec<u8>>,
    ) -> Option<Result<Vec<u8>, String>> {
        let entry = self.categories.get(&category)?;
        Some(match entry.page.as_ref()? {
            Ok(page) => page.pdf(values, photos).map_err(|error| error.message),
            Err(message) => Err(message.clone()),
        })
    }

    pub fn page_raster_png(
        &self,
        category: i64,
        values: &HashMap<String, String>,
        photos: &HashMap<String, Vec<u8>>,
        dpi: f64,
    ) -> Option<Result<Vec<u8>, String>> {
        let entry = self.categories.get(&category)?;
        Some(match entry.page.as_ref()? {
            Ok(page) => page
                .png_bytes(values, photos, dpi)
                .map_err(|error| error.message),
            Err(message) => Err(message.clone()),
        })
    }

    pub fn page_fields(&self, category: i64) -> Option<Result<Vec<String>, String>> {
        let entry = self.categories.get(&category)?;
        Some(match entry.page.as_ref()? {
            Ok(page) => Ok(page
                .index
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect()),
            Err(message) => Err(message.clone()),
        })
    }

    pub(crate) fn replace_layout(
        &mut self,
        category: i64,
        layout: Option<Result<PreparedLayout, String>>,
    ) {
        let entry = self.categories.entry(category).or_insert(CategoryEntry {
            layout: None,
            page: None,
        });
        entry.layout = layout;
    }

    pub(crate) fn replace_page(
        &mut self,
        category: i64,
        page: Option<Result<PreparedPage, String>>,
    ) {
        let entry = self.categories.entry(category).or_insert(CategoryEntry {
            layout: None,
            page: None,
        });
        entry.page = page;
    }
}

pub fn prepare_event(event_dir: &Path) -> Badges {
    let mut badges = Badges::default();
    let Ok(entries) = std::fs::read_dir(event_dir.join("badge")) else {
        return badges;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Some(category) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        let dir = entry.path();
        let layout = prepare_layout(&dir);
        let page = prepare_page(&dir);
        if layout.is_none() && page.is_none() {
            continue;
        }
        badges
            .categories
            .insert(category, CategoryEntry { layout, page });
    }
    badges
}

pub fn prepare_layout(dir: &Path) -> Option<Result<PreparedLayout, String>> {
    let cfg = dir.join("badge.cfg");
    if !cfg.is_file() {
        return None;
    }
    let bytes = match std::fs::read(&cfg) {
        Ok(bytes) => bytes,
        Err(err) => return Some(Err(err.to_string())),
    };
    if let Err(err) = ticket_render::store_layout(dir, &bytes) {
        return Some(Err(err.to_string()));
    }
    let layout = match ticket_render::decode_cfg(&bytes) {
        Ok(layout) => layout,
        Err(err) => return Some(Err(err.to_string())),
    };
    let background = match std::fs::read(dir.join("bg.png")) {
        Ok(bytes) => bytes,
        Err(err) => return Some(Err(format!("bg.png: {err}"))),
    };
    let background = match image::load_from_memory(&background) {
        Ok(image) => image.into_rgba8(),
        Err(err) => return Some(Err(format!("bg.png: {err}"))),
    };
    let mut faces = Vec::new();
    for area in &layout.areas {
        if let AreaKind::Text {
            families,
            weight,
            style,
            ..
        } = &area.kind
        {
            let face = ticket_render::capture_face(families, weight, style);
            if !faces.iter().any(|kept: &PreparedFace| {
                kept.families == face.families
                    && kept.weight == face.weight
                    && kept.style == face.style
            }) {
                faces.push(face);
            }
        }
    }
    let mut scaled = HashMap::new();
    for dpi in [203, 300] {
        match ticket_render::scale_background(&layout, &background, dpi) {
            Ok(image) => {
                scaled.insert(dpi, image);
            }
            Err(err) => return Some(Err(err.to_string())),
        }
    }
    Some(Ok(PreparedLayout {
        layout,
        background,
        faces,
        scaled,
    }))
}

pub fn prepare_page(dir: &Path) -> Option<Result<PreparedPage, String>> {
    let path = ["page.yaml", "page.yml", "page.json"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file());
    let Some(path) = path else {
        return None;
    };
    let mut page = match load_page(&path) {
        Ok(page) => page,
        Err(err) => return Some(Err(err.message)),
    };
    if let Err(err) = read_page_bytes(&mut page, dir) {
        return Some(Err(err));
    }
    let index = match index_template(&page) {
        Ok(index) => index,
        Err(err) => return Some(Err(err.message)),
    };
    let faces = FaceSet::from_page(&page);
    Some(Ok(PreparedPage { page, index, faces }))
}

fn read_page_bytes(page: &mut Page, dir: &Path) -> Result<(), String> {
    for font in page.fonts.values_mut() {
        let Some(relative) = font.source_path.as_deref() else {
            continue;
        };
        let path = dir.join(relative);
        font.bytes =
            Some(std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?);
    }
    for image in page.images.values_mut() {
        let Some(relative) = image.extracted_path.as_deref() else {
            continue;
        };
        let path = dir.join(relative);
        let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        if image
            .file_format
            .as_deref()
            .unwrap_or("")
            .eq_ignore_ascii_case("bmp")
        {
            image::load_from_memory(&bytes).map_err(|err| format!("{}: {err}", path.display()))?;
        }
        image.bytes = Some(bytes);
    }
    Ok(())
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

    mod prepare {
        use super::*;
        use crate::registration_server::current::CurrentEvent;

        const FIRST_CFG: &[u8] = include_bytes!("../../modules/ticket-render/fixtures/badge.cfg");
        const SECOND_CFG: &[u8] =
            include_bytes!("../../modules/ticket-render/fixtures/event/badge.cfg");
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        #[test]
        fn switch_prepares_cfg_and_page() {
            let (dir, current) = open();
            install(&dir, "AAA", 7, Some(FIRST_CFG), None);
            install(&dir, "AAA", 3, None, Some("name"));
            assert_eq!(current.switch_to("AAA").unwrap(), "AAA");
            assert_eq!(current.badge_layout(7).unwrap().unwrap(), size(FIRST_CFG));
            assert_eq!(
                current.badge_fields(3).unwrap().unwrap(),
                vec!["name".to_string()]
            );
            std::fs::remove_dir_all(dir.join("AAA").join("badge")).unwrap();
            assert!(current
                .badge_png(7)
                .unwrap()
                .unwrap()
                .starts_with(b"\x89PNG"));
            assert!(current
                .badge_graphic(7)
                .unwrap()
                .unwrap()
                .starts_with(b"~DG"));
            assert!(current
                .badge_page_pdf(3)
                .unwrap()
                .unwrap()
                .starts_with(b"%PDF"));
            assert!(current
                .badge_page_png(3)
                .unwrap()
                .unwrap()
                .starts_with(b"\x89PNG"));
            assert_eq!(current.badge_layout(7).unwrap().unwrap(), size(FIRST_CFG));
            assert_eq!(
                current.badge_fields(3).unwrap().unwrap(),
                vec!["name".to_string()]
            );
        }

        #[test]
        fn switch_drops_the_previous_event() {
            let (dir, current) = open();
            install(&dir, "AAA", 7, Some(FIRST_CFG), None);
            install(&dir, "BBB", 9, None, Some("city"));
            current.switch_to("AAA").unwrap();
            assert_eq!(current.switch_to("BBB").unwrap(), "BBB");
            assert!(current.badge_layout(7).is_none());
            assert_eq!(
                current.badge_fields(9).unwrap().unwrap(),
                vec!["city".to_string()]
            );
        }

        #[test]
        fn the_same_event_keeps_the_map() {
            let (dir, current) = open();
            install(&dir, "AAA", 7, Some(FIRST_CFG), None);
            install(&dir, "AAA", 3, None, Some("name"));
            current.switch_to("AAA").unwrap();
            install(&dir, "AAA", 7, Some(SECOND_CFG), None);
            install(&dir, "AAA", 3, None, Some("company"));
            assert_eq!(current.switch_to("AAA").unwrap(), "AAA");
            assert_eq!(current.badge_layout(7).unwrap().unwrap(), size(FIRST_CFG));
            assert_eq!(
                current.badge_fields(3).unwrap().unwrap(),
                vec!["name".to_string()]
            );
        }

        #[test]
        fn a_cfg_update_reloads_one_category() {
            let (dir, current) = open();
            install(&dir, "AAA", 7, Some(FIRST_CFG), None);
            install(&dir, "AAA", 3, None, Some("name"));
            current.switch_to("AAA").unwrap();
            install(&dir, "AAA", 7, Some(SECOND_CFG), None);
            current.store_badge_cfg("AAA", 7);
            assert_eq!(current.badge_layout(7).unwrap().unwrap(), size(SECOND_CFG));
            assert_eq!(
                current.badge_fields(3).unwrap().unwrap(),
                vec!["name".to_string()]
            );
        }

        #[test]
        fn a_page_update_reloads_one_category() {
            let (dir, current) = open();
            install(&dir, "AAA", 7, Some(FIRST_CFG), None);
            install(&dir, "AAA", 3, None, Some("name"));
            current.switch_to("AAA").unwrap();
            install(&dir, "AAA", 3, None, Some("company"));
            current.store_page_file("AAA", 3);
            assert_eq!(current.badge_layout(7).unwrap().unwrap(), size(FIRST_CFG));
            assert_eq!(
                current.badge_fields(3).unwrap().unwrap(),
                vec!["company".to_string()]
            );
        }

        #[test]
        fn a_broken_file_omits_that_category() {
            let (dir, current) = open();
            install(&dir, "AAA", 7, Some(&[0xac, 0xed, 0x00, 0x04]), None);
            install(&dir, "AAA", 3, None, Some("name"));
            assert_eq!(current.switch_to("AAA").unwrap(), "AAA");
            let error = current.badge_layout(7).unwrap().unwrap_err();
            assert!(error.contains("version"), "{error}");
            assert_eq!(
                current.badge_fields(3).unwrap().unwrap(),
                vec!["name".to_string()]
            );
        }

        #[test]
        fn another_event_is_not_applied() {
            let (dir, current) = open();
            install(&dir, "AAA", 7, Some(FIRST_CFG), None);
            current.switch_to("AAA").unwrap();
            install(&dir, "BBB", 7, Some(SECOND_CFG), None);
            current.store_badge_cfg("BBB", 7);
            assert_eq!(current.badge_layout(7).unwrap().unwrap(), size(FIRST_CFG));
        }

        fn open() -> (std::path::PathBuf, CurrentEvent) {
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("prepare-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let current = CurrentEvent::new(&dir);
            (dir, current)
        }

        fn install(
            base: &Path,
            event: &str,
            category: i64,
            cfg: Option<&[u8]>,
            field: Option<&str>,
        ) {
            let dir = base.join(event).join("badge").join(category.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(cfg) = cfg {
                std::fs::write(dir.join("badge.cfg"), cfg).unwrap();
                std::fs::write(dir.join("bg.png"), png_dot()).unwrap();
            }
            if let Some(field) = field {
                std::fs::write(dir.join("page.yaml"), page_yaml(field)).unwrap();
            }
        }

        fn size(cfg: &[u8]) -> (i32, i32) {
            let layout = ticket_render::decode_cfg(cfg).unwrap();
            (layout.width, layout.height)
        }

        fn page_yaml(field: &str) -> String {
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
  - id: name
    posX: 10
    posY: 10
    width: 100
    height: 20
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    template: true
    text:
      font: font1
      font_size: 12
      leading: 0
      line_height: 14
      preentered: "{{{{{field}}}}}"
      content: null
"#
            )
        }

        #[test]
        #[ignore]
        fn measure_preload() {
            let (dir, _current) = open();
            let cfg = std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("modules/ticket-render/fixtures/event/badge.cfg"),
            )
            .unwrap();
            let bg = std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("modules/ticket-render/fixtures/event/bg.png"),
            )
            .unwrap();
            let category = dir.join("AAA").join("badge").join("7");
            std::fs::create_dir_all(&category).unwrap();
            std::fs::write(category.join("badge.cfg"), &cfg).unwrap();
            std::fs::write(category.join("bg.png"), &bg).unwrap();
            let layout = ticket_render::decode_cfg(&cfg).unwrap();
            let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
            std::fs::copy(
                manifest.join("scripts/badges/tadviser.yaml"),
                category.join("page.yaml"),
            )
            .unwrap();
            let fonts = category.join("fonts");
            std::fs::create_dir_all(&fonts).unwrap();
            let acrom =
                std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Fonts");
            std::fs::copy(
                acrom.join("Acrom Regular.ttf"),
                fonts.join("Acrom Regular.ttf"),
            )
            .unwrap();
            std::fs::copy(acrom.join("Acrom Bold.ttf"), fonts.join("Acrom Bold.ttf")).unwrap();
            let values = HashMap::from([
                ("name".to_string(), "Екатерина".to_string()),
                ("surname".to_string(), "Майорова".to_string()),
                ("company".to_string(), "COLLAGENE 3D MEDICAL".to_string()),
                ("cups_name".to_string(), "ЕКАТЕРИНА".to_string()),
                ("cups_surname".to_string(), "МАЙОРОВА".to_string()),
                (
                    "cups_company".to_string(),
                    "COLLAGENE 3D MEDICAL".to_string(),
                ),
                ("position".to_string(), "менеджер".to_string()),
                ("booth-id".to_string(), "26".to_string()),
            ]);
            let photos = HashMap::new();
            let dpi = 203i32;
            let start = std::time::Instant::now();
            let prepared_layout = prepare_layout(&category).unwrap().unwrap();
            let layout_prepare_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = std::time::Instant::now();
            let prepared_page = prepare_page(&category).unwrap().unwrap();
            let page_prepare_ms = start.elapsed().as_secs_f64() * 1000.0;
            println!("prepare\tlayout\t{layout_prepare_ms:.3}");
            println!("prepare\tpage\t{page_prepare_ms:.3}");
            let px_w = layout.width as f32 * dpi as f32 / layout.dots_per_point / 72.0;
            let px_h = layout.height as f32 * dpi as f32 / layout.dots_per_point / 72.0;
            let page_w = (90.0 + 6.0) / 25.4 * dpi as f64;
            let page_h = (130.0 + 6.0) / 25.4 * dpi as f64;
            println!(
                "layout {px_w:.0}x{px_h:.0} px at {dpi} dpi, dpp {}",
                layout.dots_per_point
            );
            println!(
                "tadviser {page_w:.0}x{page_h:.0} px at {dpi} dpi, 90x130 mm plus 3 mm bleeds"
            );
            let settings = ZebraSettings::default();
            report(
                "ticket-render",
                "PNG",
                &|| {
                    let layout = ticket_render::decode_cfg(&cfg).unwrap();
                    ticket_render::render_png(&layout, &values, dpi, &bg, None).unwrap()
                },
                &|| prepared_layout.png(&values, dpi, None).unwrap(),
            );
            report(
                "ticket-render",
                "graphic",
                &|| {
                    let layout = ticket_render::decode_cfg(&cfg).unwrap();
                    ticket_render::render_graphic(&layout, &values, dpi, &bg).unwrap()
                },
                &|| prepared_layout.graphic(&values, dpi).unwrap(),
            );
            report(
                "yaml page",
                "PDF",
                &|| {
                    let page = load_page(&category.join("page.yaml")).unwrap();
                    render_visitor(&page, &values, &photos, &category).unwrap()
                },
                &|| prepared_page.pdf(&values, &photos).unwrap(),
            );
            report(
                "yaml page",
                "PNG",
                &|| {
                    let page = load_page(&category.join("page.yaml")).unwrap();
                    let page = prepare_visitor(&page, &values, &photos, &category).unwrap();
                    render_raster(&page, dpi as f64, &category).unwrap().png
                },
                &|| {
                    prepared_page
                        .png_bytes(&values, &photos, dpi as f64)
                        .unwrap()
                },
            );
            report(
                "yaml page",
                "BMP",
                &|| {
                    let page = load_page(&category.join("page.yaml")).unwrap();
                    let page = prepare_visitor(&page, &values, &photos, &category).unwrap();
                    render_raster(&page, dpi as f64, &category).unwrap().bmp
                },
                &|| {
                    prepared_page
                        .bmp_bytes(&values, &photos, dpi as f64)
                        .unwrap()
                },
            );
            report(
                "yaml page",
                "ZPL",
                &|| page_zpl(&category, &values, &photos, dpi as f64, 4.1, &settings).unwrap(),
                &|| {
                    let (payload, width) =
                        prepared_page.graphic(&values, &photos, dpi as f64).unwrap();
                    wrap_zpl(
                        &payload,
                        width,
                        (4.1 * dpi as f64).round() as i64,
                        &settings,
                    )
                },
            );
        }

        fn report(
            renderer: &str,
            output: &str,
            full: &dyn Fn() -> Vec<u8>,
            prebuilt: &dyn Fn() -> Vec<u8>,
        ) {
            let full_ms = samples(full);
            let pre_ms = samples(prebuilt);
            let full_again = samples(full);
            let (full_p25, full_med, full_p75) = spread(&full_ms, &full_again);
            let (pre_p25, pre_med, pre_p75) = spread(&pre_ms, &samples(prebuilt));
            println!(
                "{renderer}\t{output}\t{full_med:.3}\t{full_p25:.3}\t{full_p75:.3}\t{pre_med:.3}\t{pre_p25:.3}\t{pre_p75:.3}\t{:.3}",
                full_med / pre_med
            );
        }

        fn samples(draw: &dyn Fn() -> Vec<u8>) -> Vec<f64> {
            const WARM: usize = 3;
            const N: usize = 30;
            for _ in 0..WARM {
                std::hint::black_box(draw());
            }
            let mut times = Vec::with_capacity(N);
            for _ in 0..N {
                let start = std::time::Instant::now();
                std::hint::black_box(draw());
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            times
        }

        fn spread(first: &[f64], second: &[f64]) -> (f64, f64, f64) {
            let mut times = first.to_vec();
            times.extend_from_slice(second);
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = times.len();
            (times[n / 4], times[n / 2], times[n * 3 / 4])
        }

        fn png_dot() -> Vec<u8> {
            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            bytes
        }
    }
}
