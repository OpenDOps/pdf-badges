use std::path::PathBuf;
use std::sync::Mutex;

use lopdf::{Document, Object};

use rust_reg::struct_to_pdf::{
    authored_text_bounds, load_page, parse_page, pdf_point, render_page, render_pages, save_pdf,
    to_points, ContentEntry, HorizontalAlign, RenderError, SourceFormat, Units, VerticalAlign,
};

const CARD_TEXT: &str = "<span font=\"font1\">{{name}} carried the lantern along the river road past the mill and the evening market and the stone bridge and the quiet school and the old bakery {{name}} carried the lantern along the river road past the mill and the evening market and the stone bridge and the quiet school and the old bakery<br>{{surname}} kept every letter in a wooden box beside the window and read them again each winter evening under the lamp and wrote the street names in a careful hand {{surname}} kept every letter in a wooden box beside the window and read them again each winter evening under the lamp and wrote the street names in a careful hand</span><br><br><span font=\"font2\"><bold>{{company name}} prints the posters and folds every sheet before the morning train leaves the station and stacks the bundles by the door {{company name}} prints the posters and folds every sheet before the morning train leaves the station and stacks the bundles by the door</bold></span>";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/struct-to-pdf")
        .join(name)
}

fn parse_yaml(source: &str) -> Result<rust_reg::struct_to_pdf::Page, RenderError> {
    parse_page(source, SourceFormat::Yaml)
}

fn assert_context(result: Result<rust_reg::struct_to_pdf::Page, RenderError>, needle: &str) {
    let err = result.expect_err("expected a validation error");
    assert!(
        err.context.contains(needle),
        "context {:?} does not contain {needle}; message: {}",
        err.context,
        err.message
    );
}

fn minimal_page() -> String {
    r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 3, right: 3, bottom: 3, left: 3}
resources:
  images: {}
  fonts: {}
contents:
  - id: box-1
    posX: 10
    posY: 30
    width: 70
    height: 50
    alignment:
      horizontal: left
      vertical: top
    padding: {top: 0, right: 0, bottom: 0, left: 0}
    text:
      font: font1
      font_size: 5
      leading: 0.5
      line_height: 6
      preentered: hello
      content: null
"#
    .to_string()
}

#[test]
fn loads_card_yaml() {
    let page = load_page(&fixture("card.yaml")).expect("card yaml");
    assert_eq!(page.page_size.width, 90.0);
    assert_eq!(page.page_size.height, 130.0);
    assert_eq!(page.page_size.units, Units::Mm);
    assert_eq!(page.page_size.bleeds.top, 3.0);
    assert_eq!(page.page_size.bleeds.right, 3.0);
    assert_eq!(page.page_size.bleeds.bottom, 3.0);
    assert_eq!(page.page_size.bleeds.left, 3.0);
    assert_eq!(page.contents.len(), 1);
    match &page.contents[0] {
        ContentEntry::Text(text) => {
            assert_eq!(text.id, "card-text");
            assert!(text.auto_scale);
            assert_eq!(text.text.preentered.as_deref(), Some(CARD_TEXT));
            assert_eq!(text.text.content, None);
            assert_eq!(text.horizontal, HorizontalAlign::Center);
            assert_eq!(text.vertical, VerticalAlign::Middle);
        }
        ContentEntry::Image(_) => panic!("card entry is a text box"),
    }
}

#[test]
fn loads_same_page_as_json() {
    let yaml = load_page(&fixture("card.yaml")).unwrap();
    let json = load_page(&fixture("card.json")).unwrap();
    assert_eq!(yaml, json);
}

#[test]
fn rejects_document() {
    let source = "pages: []\nmetadata: {}\n";
    assert_context(parse_yaml(source), "pages");
}

#[test]
fn rejects_missing_bleeds() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
resources:
  images: {}
  fonts: {}
contents: []
"#;
    assert_context(parse_yaml(source), "bleeds");
}

#[test]
fn rejects_negative_bleed() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 3, right: 3, bottom: 3, left: -1}
resources:
  images: {}
  fonts: {}
contents: []
"#;
    assert_context(parse_yaml(source), "bleeds.left");
}

#[test]
fn rejects_bad_units() {
    let mut source = minimal_page();
    source = source.replace("units: mm", "units: inches");
    assert_context(parse_yaml(&source), "units");
}

#[test]
fn rejects_entry_with_no_kind() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 0, right: 0, bottom: 0, left: 0}
resources:
  images: {}
  fonts: {}
contents:
  - id: kindless
    posX: 0
    posY: 0
    width: 10
    height: 10
"#;
    assert_context(parse_yaml(source), "kindless");
}

#[test]
fn rejects_missing_id() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 0, right: 0, bottom: 0, left: 0}
resources:
  images: {}
  fonts: {}
contents:
  - posX: 0
    posY: 0
    width: 10
    height: 10
    alignment: {horizontal: left, vertical: top}
    padding: {top: 0, right: 0, bottom: 0, left: 0}
    text:
      font: font1
      font_size: 5
      leading: 0
      line_height: 6
"#;
    assert_context(parse_yaml(source), "id");
}

#[test]
fn rejects_bad_horizontal() {
    let source = minimal_page().replace("horizontal: left", "horizontal: justified");
    assert_context(parse_yaml(&source), "box-1");
}

#[test]
fn rejects_bad_vertical() {
    let source = minimal_page().replace("vertical: top", "vertical: center");
    assert_context(parse_yaml(&source), "box-1");
}

#[test]
fn rejects_padding_wider_than_frame() {
    let source = minimal_page().replace(
        "padding: {top: 0, right: 0, bottom: 0, left: 0}",
        "padding: {top: 0, right: 40, bottom: 0, left: 40}",
    );
    assert_context(parse_yaml(&source), "box-1");
}

#[test]
fn rejects_missing_font() {
    let source = minimal_page().replace("      font: font1\n", "");
    assert_context(parse_yaml(&source), "font");
}

#[test]
fn rejects_missing_leading() {
    let source = minimal_page().replace("      leading: 0.5\n", "");
    assert_context(parse_yaml(&source), "leading");
}

#[test]
fn rejects_missing_line_height() {
    let source = minimal_page().replace("      line_height: 6\n", "");
    assert_context(parse_yaml(&source), "line_height");
}

#[test]
fn rejects_image_without_resource() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 0, right: 0, bottom: 0, left: 0}
resources:
  images: {}
  fonts: {}
contents:
  - id: img-1
    posX: 0
    posY: 0
    width: 10
    height: 10
    image: {}
"#;
    assert_context(parse_yaml(source), "img-1");
}

fn blank_page_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/blank-page.pdf")
}

fn approx_eq(actual: [f32; 4], expected: [f32; 4]) {
    for (got, want) in actual.iter().zip(expected) {
        assert!((got - want).abs() < 0.01, "{actual:?} != {expected:?}");
    }
}

fn page_boxes(bytes: &[u8]) -> (lopdf::Dictionary, usize) {
    let doc = Document::load_mem(bytes).expect("pdf bytes");
    let pages = doc.get_pages();
    let page_id = pages.get(&1).expect("one page numbered 1");
    let dict = doc.get_dictionary(*page_id).expect("page dict").clone();
    (dict, pages.len())
}

fn rect_of(dict: &lopdf::Dictionary, key: &[u8]) -> [f32; 4] {
    let arr = dict.get(key).expect("box").as_array().expect("array");
    let mut out = [0.0; 4];
    for (index, object) in arr.iter().enumerate() {
        out[index] = match object {
            Object::Real(value) => *value,
            Object::Integer(value) => *value as f32,
            other => panic!("unexpected box entry {other:?}"),
        };
    }
    out
}

fn mm(value: f32) -> f32 {
    value * 72.0 / 25.4
}

#[test]
fn mm_to_points() {
    assert!((to_points(25.4, Units::Mm) - 72.0).abs() < 0.01);
    assert!((to_points(72.0, Units::Points) - 72.0).abs() < 0.01);
}

#[test]
fn card_media_and_trim() {
    let mut page = load_page(&fixture("card.yaml")).unwrap();
    page.contents.clear();
    let bytes = render_page(&page, &fixture("")).unwrap();
    let out = blank_page_path();
    save_pdf(&bytes, &out).unwrap();

    let (dict, count) = page_boxes(&bytes);
    assert_eq!(count, 1);
    assert!(dict.get(b"UserUnit").is_err());
    let media = [0.0, 0.0, mm(96.0), mm(136.0)];
    approx_eq(rect_of(&dict, b"MediaBox"), media);
    approx_eq(rect_of(&dict, b"BleedBox"), media);
    approx_eq(rect_of(&dict, b"CropBox"), media);
    approx_eq(
        rect_of(&dict, b"TrimBox"),
        [mm(3.0), mm(3.0), mm(93.0), mm(133.0)],
    );
}

#[test]
fn zero_bleed_trim_equals_media() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 0, right: 0, bottom: 0, left: 0}
resources:
  images: {}
  fonts: {}
contents: []
"#;
    let page = parse_yaml(source).unwrap();
    let bytes = render_page(&page, &fixture("")).unwrap();
    let (dict, _) = page_boxes(&bytes);
    let media = [0.0, 0.0, mm(90.0), mm(110.0)];
    approx_eq(rect_of(&dict, b"MediaBox"), media);
    approx_eq(rect_of(&dict, b"TrimBox"), media);
}

#[test]
fn points_input_is_not_scaled() {
    let source = r#"
page_size:
  width: 200
  height: 100
  units: points
  bleeds: {top: 10, right: 10, bottom: 10, left: 10}
resources:
  images: {}
  fonts: {}
contents: []
"#;
    let page = parse_yaml(source).unwrap();
    let bytes = render_page(&page, &fixture("")).unwrap();
    let (dict, _) = page_boxes(&bytes);
    approx_eq(rect_of(&dict, b"MediaBox"), [0.0, 0.0, 220.0, 120.0]);
    approx_eq(rect_of(&dict, b"TrimBox"), [10.0, 10.0, 210.0, 110.0]);
}

#[test]
fn non_empty_contents_not_yet() {
    let (page, base_dir) = load_fixture("card.yaml");
    let text = page_text(&render_page(&page, &base_dir).expect("card draws"));
    assert!(text.contains("{{name}}"));
    assert!(text.contains("{{surname}}"));
    assert!(text.contains("{{company name}}"));
}

#[test]
fn rejects_negative_box_width() {
    let source = minimal_page().replace("width: 70", "width: -1");
    assert_context(parse_yaml(&source), "box-1");
}

fn load_fixture(name: &str) -> (rust_reg::struct_to_pdf::Page, PathBuf) {
    let path = fixture(name);
    let page = load_page(&path).unwrap_or_else(|err| panic!("load {name}: {err}"));
    let base_dir = path.parent().expect("fixture directory").to_path_buf();
    (page, base_dir)
}

fn image_xobject(bytes: &[u8]) -> (lopdf::Dictionary, Vec<u8>) {
    let doc = Document::load_mem(bytes).unwrap();
    let page_id = *doc.get_pages().get(&1).unwrap();
    let resources = doc
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap();
    let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
    let image_id = xobjects.get(b"Im1").unwrap().as_reference().unwrap();
    let stream = doc.get_object(image_id).unwrap().as_stream().unwrap();
    (stream.dict.clone(), stream.content.clone())
}

fn content_ops(bytes: &[u8]) -> Vec<lopdf::content::Operation> {
    let doc = Document::load_mem(bytes).expect("pdf bytes");
    let page_id = *doc.get_pages().get(&1).expect("one page");
    doc.get_and_decode_page_content(page_id)
        .expect("content")
        .operations
}

fn num(object: &Object) -> f32 {
    match object {
        Object::Real(value) => *value,
        Object::Integer(value) => *value as f32,
        other => panic!("unexpected number {other:?}"),
    }
}

fn approx(got: f32, want: f32) {
    assert!((got - want).abs() < 0.01, "{got} != {want}");
}

#[test]
fn jpeg_rect_inside_trim() {
    let (page, base_dir) = load_fixture("jpeg-page.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/jpeg-page.pdf");
    save_pdf(&bytes, &out).unwrap();

    let ops = content_ops(&bytes);
    let cm = ops.iter().find(|op| op.operator == "cm").expect("cm");
    let width = num(&cm.operands[0]);
    let height = num(&cm.operands[3]);
    let pdf_x = num(&cm.operands[4]);
    let pdf_y = num(&cm.operands[5]);
    let bleed = mm(3.0);
    let trim_height = mm(110.0);
    approx(pdf_x - bleed, mm(10.0));
    approx(trim_height - ((pdf_y + height) - bleed), mm(20.0));
    approx(width, mm(30.0));
    approx(height, mm(40.0));
    assert!(ops.iter().any(|op| op.operator == "Do"));

    let doc = Document::load_mem(&bytes).unwrap();
    let page_id = *doc.get_pages().get(&1).unwrap();
    let resources = doc
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap();
    let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
    let image_id = xobjects.get(b"Im1").unwrap().as_reference().unwrap();
    let stream = doc.get_object(image_id).unwrap().as_stream().unwrap();
    assert_eq!(
        stream.dict.get(b"Filter").unwrap().as_name().unwrap(),
        b"DCTDecode"
    );
    assert_eq!(
        stream.dict.get(b"Subtype").unwrap().as_name().unwrap(),
        b"Image"
    );
    assert_eq!(
        stream.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
        b"DeviceRGB"
    );
    assert_eq!(stream.content[..2], [0xFF, 0xD8]);
}

#[test]
fn jpeg_in_left_bleed() {
    let (page, base_dir) = load_fixture("jpeg-bleed.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let ops = content_ops(&bytes);
    let cm = ops.iter().find(|op| op.operator == "cm").expect("cm");
    approx(num(&cm.operands[4]), mm(1.0));
    assert!(ops.iter().any(|op| op.operator == "Do"));
}

#[test]
fn jpeg_missing_file() {
    let (page, base_dir) = load_fixture("jpeg-missing.yaml");
    let err = render_page(&page, &base_dir).expect_err("missing file");
    assert!(err.context.contains("photo-1"));
}

#[test]
fn jpeg_unknown_name() {
    let (page, base_dir) = load_fixture("jpeg-unknown.yaml");
    let err = render_page(&page, &base_dir).expect_err("unknown name");
    assert!(err.context.contains("photo-1"));
}

#[test]
fn bmp_matches_jpeg_rect() {
    let (jpeg_page, jpeg_dir) = load_fixture("jpeg-page.yaml");
    let jpeg = render_page(&jpeg_page, &jpeg_dir).unwrap();
    let (bmp_page, bmp_dir) = load_fixture("jpeg-bmp.yaml");
    let bmp = render_page(&bmp_page, &bmp_dir).unwrap();
    let jpeg_cm = content_ops(&jpeg)
        .into_iter()
        .find(|op| op.operator == "cm")
        .expect("jpeg cm");
    let bmp_cm = content_ops(&bmp)
        .into_iter()
        .find(|op| op.operator == "cm")
        .expect("bmp cm");
    for index in [0, 3, 4, 5] {
        approx(num(&bmp_cm.operands[index]), num(&jpeg_cm.operands[index]));
    }
}

#[test]
fn bmp_rgb_color_space() {
    let (page, base_dir) = load_fixture("jpeg-bmp.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let (dict, samples) = image_xobject(&bytes);
    assert!(dict.get(b"Filter").is_err());
    assert_eq!(
        dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
        b"DeviceRGB"
    );
    assert_eq!(dict.get(b"Width").unwrap().as_i64().unwrap(), 8);
    assert_eq!(dict.get(b"Height").unwrap().as_i64().unwrap(), 8);
    assert_eq!(&samples[..3], &[255, 0, 0]);
    assert_eq!(samples.len(), 8 * 8 * 3);
}

#[test]
fn bmp_32bit_drops_alpha() {
    let (page, base_dir) = load_fixture("bmp-32.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let (dict, samples) = image_xobject(&bytes);
    assert_eq!(
        dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
        b"DeviceRGB"
    );
    assert!(dict.get(b"Filter").is_err());
    assert_eq!(&samples[..3], &[255, 0, 0]);
    assert_eq!(&samples[3..6], &[0, 255, 0]);
    assert_eq!(samples.len(), 8 * 8 * 3);
}

#[test]
fn bmp_gray() {
    let (page, base_dir) = load_fixture("bmp-gray.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let (dict, samples) = image_xobject(&bytes);
    assert_eq!(
        dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
        b"DeviceGray"
    );
    assert!(dict.get(b"Filter").is_err());
    assert_eq!(samples, vec![200, 10, 10, 200]);
}

#[test]
fn bmp_bad_depth() {
    let (page, base_dir) = load_fixture("bmp-1.yaml");
    let out = card_out("bmp-1.pdf");
    let _ = std::fs::remove_file(&out);
    let err = render_page(&page, &base_dir).expect_err("1-bit bmp");
    assert!(err.context.contains("photo-1"), "{}", err.context);
    assert!(err.message.contains('1'));
    assert!(!out.exists());
}

#[test]
fn jp2_still_rejected() {
    let (page, base_dir) = load_fixture("jpeg-jp2.yaml");
    let err = render_page(&page, &base_dir).expect_err("jp2");
    assert!(err.context.contains("photo-1"));
    assert!(err.message.contains("not in v0 yet"));
}

fn page_text(bytes: &[u8]) -> String {
    let doc = Document::load_mem(bytes).expect("pdf bytes");
    doc.extract_text(&[1]).expect("extract text")
}

#[test]
fn text_and_image() {
    let (page, base_dir) = load_fixture("text-and-image.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let out =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/text-and-image.pdf");
    save_pdf(&bytes, &out).unwrap();

    assert!(page_text(&bytes).contains("Hello"));

    let ops = content_ops(&bytes);
    let cm = ops.iter().find(|op| op.operator == "cm").expect("cm");
    let width = num(&cm.operands[0]);
    let height = num(&cm.operands[3]);
    let pdf_x = num(&cm.operands[4]);
    let pdf_y = num(&cm.operands[5]);
    let bleed = mm(3.0);
    let trim_height = mm(110.0);
    approx(pdf_x - bleed, mm(10.0));
    approx(trim_height - ((pdf_y + height) - bleed), mm(15.0));
    approx(width, mm(30.0));
    approx(height, mm(40.0));

    let do_at = ops.iter().position(|op| op.operator == "Do").expect("Do");
    let tj_at = ops.iter().position(|op| op.operator == "Tj").expect("Tj");
    assert!(do_at < tj_at);
}

#[test]
fn plain_ascii_roundtrip() {
    let (page, base_dir) = load_fixture("text-hello.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/text-hello.pdf");
    save_pdf(&bytes, &out).unwrap();
    assert!(page_text(&bytes).contains("Hello"));
}

#[test]
fn content_overrides_preentered() {
    let (page, base_dir) = load_fixture("text-override.yaml");
    let text = page_text(&render_page(&page, &base_dir).unwrap());
    assert!(text.contains("new"));
    assert!(!text.contains("old"));
}

#[test]
fn empty_content_draws_nothing() {
    let (page, base_dir) = load_fixture("text-empty.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let doc = Document::load_mem(&bytes).expect("pdf bytes");
    assert_eq!(doc.get_pages().len(), 1);
    assert_eq!(page_text(&bytes), "");
}

#[test]
fn missing_both_strings() {
    let (page, base_dir) = load_fixture("text-missing-strings.yaml");
    let err = render_page(&page, &base_dir).expect_err("missing strings");
    assert!(err.context.contains("line-1"));
}

#[test]
fn missing_font_file() {
    let (page, base_dir) = load_fixture("text-missing-font.yaml");
    let out =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/missing-font.pdf");
    let _ = std::fs::remove_file(&out);
    let err = render_page(&page, &base_dir).expect_err("missing font");
    assert!(err.context.contains("line-1"));
    assert!(!out.exists());
}

#[test]
fn unknown_font_key() {
    let (page, base_dir) = load_fixture("text-unknown-font.yaml");
    let err = render_page(&page, &base_dir).expect_err("unknown font");
    assert!(err.context.contains("line-1"));
}

#[test]
fn markup_rejected_until_step_6() {
    let (page, base_dir) = load_fixture("text-markup.yaml");
    let err = render_page(&page, &base_dir).expect_err("font2 is not on this page");
    assert!(err.context.contains("line-1"));
    assert!(err.message.contains("font2"));
}

#[test]
fn rotation_90_points_the_baseline_up() {
    let yaml = r#"
page_size:
  width: 50
  height: 70
  units: mm
  bleeds: {top: 0, right: 0, bottom: 0, left: 0}
resources:
  images: {}
  fonts:
    font1:
      name: font1
      font_family: LiberationSerif
      weight: bold
      italic: false
      embedded: true
      source_path: fonts/LiberationSerif-Bold.ttf
contents:
  - id: company
    posX: 2
    posY: 3
    width: 20
    height: 64
    rotation: 90
    alignment: {horizontal: center, vertical: middle}
    padding: {top: 0, right: 0, bottom: 0, left: 0}
    text:
      font: font1
      font_size: 5
      leading: 0
      line_height: 6
      preentered: Hi
      content: null
"#;
    let page = parse_yaml(yaml).expect("page");
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/struct-to-pdf");
    let bytes = render_page(&page, &base).unwrap();
    let tm = content_ops(&bytes)
        .into_iter()
        .find(|op| op.operator == "Tm")
        .expect("Tm");
    approx(num(&tm.operands[0]), 0.0);
    approx(num(&tm.operands[1]), 1.0);
    approx(num(&tm.operands[2]), -1.0);
    approx(num(&tm.operands[3]), 0.0);
}

fn baseline_at_top_plus_ascent() {
    let (page, base_dir) = load_fixture("text-baseline.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = ttf_parser::Face::parse(&font_bytes, 0).unwrap();
    let ascent = 5.0 * f64::from(face.ascender()) / f64::from(face.units_per_em());
    let (_, pdf_y) = pdf_point(&page, 0.0, 30.0 + ascent);
    let tm = content_ops(&bytes)
        .into_iter()
        .find(|op| op.operator == "Tm")
        .expect("Tm");
    approx(num(&tm.operands[5]), pdf_y);
}

#[test]
fn cyrillic_roundtrip() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = ttf_parser::Face::parse(&font_bytes, 0).unwrap();
    for ch in "Сергей".chars() {
        assert!(
            face.glyph_index(ch).is_some(),
            "Liberation Serif has no glyph for {ch}"
        );
    }

    let (page, base_dir) = load_fixture("text-cyrillic.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let out =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/text-cyrillic.pdf");
    save_pdf(&bytes, &out).unwrap();
    assert_eq!(page_text(&bytes).trim_end(), "Сергей");
}

struct WarnCapture;

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOGGER: WarnCapture = WarnCapture;

impl log::Log for WarnCapture {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            WARNINGS.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

#[test]
fn cyrillic_warns_on_latin_font() {
    let font_bytes = std::fs::read(fixture("fonts/LatinOnly.ttf")).unwrap();
    let face = ttf_parser::Face::parse(&font_bytes, 0).unwrap();
    assert!(face.glyph_index('A').is_some());
    for ch in "Сергей".chars() {
        assert!(
            face.glyph_index(ch).is_none(),
            "LatinOnly unexpectedly covers {ch}"
        );
    }

    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);
    WARNINGS.lock().unwrap().clear();

    let (page, base_dir) = load_fixture("text-cyrillic-latin-font.yaml");
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/struct-to-pdf/cyrillic-latin-font.pdf");
    let _ = std::fs::remove_file(&out);
    let err = render_page(&page, &base_dir).expect_err("latin font cannot draw cyrillic");
    assert!(err.context.contains("line-1"));
    let warnings = WARNINGS.lock().unwrap().clone();
    let warning = warnings
        .iter()
        .find(|line| line.contains("line-1"))
        .unwrap_or_else(|| panic!("warnings: {warnings:?}"));
    for ch in "Сергей".chars() {
        let code = format!("U+{:04X}", u32::from(ch));
        assert!(
            warning.contains(&code) && err.message.contains(&code),
            "missing {code} in warning {warning:?}"
        );
    }
    assert!(!out.exists());
}

fn measure_pt(face: &ttf_parser::Face<'_>, text: &str, font_size_pt: f32) -> f32 {
    let units_per_em = f32::from(face.units_per_em());
    text.chars()
        .map(|ch| {
            let glyph = face
                .glyph_index(ch)
                .unwrap_or_else(|| panic!("missing {ch}"));
            let advance = face.glyph_hor_advance(glyph).unwrap_or(0);
            f32::from(advance) * font_size_pt / units_per_em
        })
        .sum()
}

fn liberation_face(bytes: &[u8]) -> ttf_parser::Face<'_> {
    ttf_parser::Face::parse(bytes, 0).expect("Liberation Serif")
}

fn document_point(page: &rust_reg::struct_to_pdf::Page, pdf_x: f32, pdf_y: f32) -> (f32, f32) {
    let units = page.page_size.units;
    let x = pdf_x - to_points(page.page_size.bleeds.left, units);
    let y = to_points(page.page_size.height, units)
        - (pdf_y - to_points(page.page_size.bleeds.bottom, units));
    (x, y)
}

fn text_box(page: &rust_reg::struct_to_pdf::Page) -> &rust_reg::struct_to_pdf::TextBox {
    match &page.contents[0] {
        ContentEntry::Text(text) => text,
        ContentEntry::Image(_) => panic!("expected a text box"),
    }
}

fn baselines(bytes: &[u8]) -> Vec<(f32, f32)> {
    content_ops(bytes)
        .iter()
        .filter(|op| op.operator == "Tm")
        .map(|op| (num(&op.operands[4]), num(&op.operands[5])))
        .collect()
}

#[test]
fn wraps_on_spaces() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font_bytes);
    let size = to_points(5.0, Units::Mm);
    let two = measure_pt(&face, "one two", size);
    let three = measure_pt(&face, "one two three", size);
    let content = to_points(22.0, Units::Mm);
    assert!(
        two <= content + 0.01 && three > content + 0.01,
        "one two is {two} pt, one two three is {three} pt, content width is {content} pt"
    );

    let (page, base_dir) = load_fixture("text-wrap.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let extracted = page_text(&bytes);
    let lines: Vec<&str> = extracted
        .split('\n')
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(lines, vec!["one two", "three"]);
}

#[test]
fn long_word_stays_intact() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font_bytes);
    let word = "Supercalifragilistic";
    let word_width = measure_pt(&face, word, to_points(12.0, Units::Mm));
    assert!(word_width > to_points(8.0, Units::Mm));

    let (page, base_dir) = load_fixture("text-long-word.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let tj_count = content_ops(&bytes)
        .iter()
        .filter(|op| op.operator == "Tj")
        .count();
    assert_eq!(tj_count, 1);
    assert_eq!(page_text(&bytes).trim_end(), word);
}

#[test]
fn left_top() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font_bytes);
    let (page, base_dir) = load_fixture("text-left-top.yaml");
    let text = text_box(&page);
    let bytes = render_page(&page, &base_dir).unwrap();
    let points = baselines(&bytes);
    assert_eq!(points.len(), 2);
    let (doc_x, baseline_y) = document_point(&page, points[0].0, points[0].1);
    let units = page.page_size.units;
    approx(doc_x, to_points(text.pos_x, units));
    let ascent = to_points(text.text.font_size, units) * f32::from(face.ascender())
        / f32::from(face.units_per_em());
    approx(baseline_y - ascent, to_points(text.pos_y, units));
}

#[test]
fn right_edge() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font_bytes);
    let (page, base_dir) = load_fixture("text-right.yaml");
    let text = text_box(&page);
    let bytes = render_page(&page, &base_dir).unwrap();
    let points = baselines(&bytes);
    assert_eq!(points.len(), 1);
    let (doc_x, _) = document_point(&page, points[0].0, points[0].1);
    let units = page.page_size.units;
    let width = measure_pt(&face, "Hello", to_points(text.text.font_size, units));
    approx(doc_x + width, to_points(text.pos_x + text.width, units));
}

#[test]
fn bottom_edge() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font_bytes);
    let (page, base_dir) = load_fixture("text-bottom.yaml");
    let text = text_box(&page);
    let bytes = render_page(&page, &base_dir).unwrap();
    let points = baselines(&bytes);
    assert_eq!(points.len(), 2);
    let (_, last_baseline) = document_point(&page, points[1].0, points[1].1);
    let units = page.page_size.units;
    let descent = to_points(text.text.font_size, units) * f32::from(-face.descender())
        / f32::from(face.units_per_em());
    approx(
        last_baseline + descent,
        to_points(text.pos_y + text.height, units),
    );
}

#[test]
fn center_middle_in_frame() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font_bytes);
    let (page, base_dir) = load_fixture("text-center.yaml");
    let text = text_box(&page);
    let bytes = render_page(&page, &base_dir).unwrap();
    let out =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/text-center.pdf");
    save_pdf(&bytes, &out).unwrap();

    let points = baselines(&bytes);
    assert_eq!(points.len(), 2);
    let units = page.page_size.units;
    let center_x = to_points(text.pos_x + text.width / 2.0, units);
    for (shown, point) in ["Hi", "Yo"].into_iter().zip(points.iter()) {
        let (doc_x, _) = document_point(&page, point.0, point.1);
        let width = measure_pt(&face, shown, to_points(text.text.font_size, units));
        approx(doc_x + width / 2.0, center_x);
    }

    let (_, first_baseline) = document_point(&page, points[0].0, points[0].1);
    let ascent = to_points(text.text.font_size, units) * f32::from(face.ascender())
        / f32::from(face.units_per_em());
    let block_height = to_points(2.0 * text.text.line_height + text.text.leading, units);
    let block_center = (first_baseline - ascent) + block_height / 2.0;
    approx(
        block_center,
        to_points(text.pos_y + text.height / 2.0, units),
    );
}

#[test]
fn padding_insets_left() {
    let (page, base_dir) = load_fixture("text-padding.yaml");
    let text = text_box(&page);
    let bytes = render_page(&page, &base_dir).unwrap();
    let points = baselines(&bytes);
    assert_eq!(points.len(), 1);
    let (doc_x, _) = document_point(&page, points[0].0, points[0].1);
    approx(
        doc_x,
        to_points(text.pos_x + text.padding.left, page.page_size.units),
    );
}

#[test]
fn leading_adds_to_line_height() {
    let (page, base_dir) = load_fixture("text-leading.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let points = baselines(&bytes);
    assert_eq!(points.len(), 2);
    approx((points[0].1 - points[1].1).abs(), to_points(6.5, Units::Mm));
}

#[test]
fn underline_strokes() {
    let font_bytes = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font_bytes);
    let (page, base_dir) = load_fixture("text-underline.yaml");
    let text = text_box(&page);
    let bytes = render_page(&page, &base_dir).unwrap();
    let ops = content_ops(&bytes);
    let start = ops
        .iter()
        .find(|op| op.operator == "m")
        .expect("underline start");
    let end = ops
        .iter()
        .find(|op| op.operator == "l")
        .expect("underline end");
    let width = measure_pt(
        &face,
        "Hello",
        to_points(text.text.font_size, page.page_size.units),
    );
    approx(
        (num(&end.operands[0]) - num(&start.operands[0])).abs(),
        width,
    );
    assert!(ops.iter().any(|op| op.operator == "S"));
}

fn render_card() -> Vec<u8> {
    let (page, base_dir) = load_fixture("card.yaml");
    render_page(&page, &base_dir).expect("card")
}

fn card_out(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/struct-to-pdf")
        .join(name)
}

#[test]
fn card_pdf_is_written() {
    let bytes = render_card();
    let out = card_out("card.pdf");
    save_pdf(&bytes, &out).unwrap();
    assert!(out.exists());
    assert!(bytes.starts_with(b"%PDF"));
    let doc = Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.get_pages().len(), 1);
}

#[test]
fn card_trim_and_bleeds() {
    let (dict, _) = page_boxes(&render_card());
    let media = rect_of(&dict, b"MediaBox");
    let trim = rect_of(&dict, b"TrimBox");
    approx(trim[2] - trim[0], mm(90.0));
    approx(trim[3] - trim[1], mm(130.0));
    approx(trim[0] - media[0], mm(3.0));
    approx(media[2] - trim[2], mm(3.0));
    approx(trim[1] - media[1], mm(3.0));
    approx(media[3] - trim[3], mm(3.0));
}

#[test]
fn card_placeholders_present() {
    let text = page_text(&render_card());
    assert!(text.contains("{{name}}"));
    assert!(text.contains("{{surname}}"));
    assert!(text.contains("{{company name}}"));
    assert!(!text.contains("span"));
    assert!(!text.contains("bold"));
    assert!(!text.contains("font1"));
}

fn font_program(bytes: &[u8], resource: &str) -> Vec<u8> {
    let doc = Document::load_mem(bytes).unwrap();
    let page_id = *doc.get_pages().get(&1).unwrap();
    let resources = doc
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap();
    let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
    let type0_id = fonts
        .get(resource.as_bytes())
        .unwrap()
        .as_reference()
        .unwrap();
    let descendants = doc
        .get_dictionary(type0_id)
        .unwrap()
        .get(b"DescendantFonts")
        .unwrap()
        .as_array()
        .unwrap();
    let cid_id = descendants[0].as_reference().unwrap();
    let descriptor_id = doc
        .get_dictionary(cid_id)
        .unwrap()
        .get(b"FontDescriptor")
        .unwrap()
        .as_reference()
        .unwrap();
    let file_id = doc
        .get_dictionary(descriptor_id)
        .unwrap()
        .get(b"FontFile2")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object(file_id)
        .unwrap()
        .as_stream()
        .unwrap()
        .content
        .clone()
}

fn tj_fonts(bytes: &[u8]) -> Vec<String> {
    let mut current = String::new();
    let mut used = Vec::new();
    for op in content_ops(bytes) {
        if op.operator == "Tf" {
            current = String::from_utf8(op.operands[0].as_name().unwrap().to_vec()).unwrap();
        }
        if op.operator == "Tj" {
            used.push(current.clone());
        }
    }
    used
}

#[test]
fn card_two_fonts() {
    let bytes = render_card();
    let used = tj_fonts(&bytes);
    assert!(used.len() >= 2);
    let regular = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let bold = std::fs::read(fixture("fonts/LiberationSerif-Bold.ttf")).unwrap();
    assert_eq!(font_program(&bytes, &used[0]), regular);
    assert_eq!(font_program(&bytes, used.last().unwrap()), bold);
    assert_ne!(used[0], *used.last().unwrap());
}

#[test]
fn card_long_text_shrinks() {
    let (page, base_dir) = load_fixture("card.yaml");
    let text = text_box(&page);
    let (unscaled_width, unscaled_height) = authored_text_bounds(&page, text, &base_dir).unwrap();
    assert!(
        unscaled_height > 50.0,
        "unscaled block height {unscaled_height} mm already fits"
    );
    let height_scale = 50.0 / unscaled_height;
    let width_scale = 70.0 / unscaled_width;
    assert!(
        height_scale <= width_scale,
        "width ratio {width_scale} is tighter than height ratio {height_scale}"
    );

    let bytes = render_page(&page, &base_dir).unwrap();
    let extracted = page_text(&bytes);
    assert!(extracted.contains("{{name}}"));
    assert!(extracted.contains("{{surname}}"));
    assert!(extracted.contains("{{company name}}"));

    let scale = height_scale as f32;
    for size in tf_sizes(&bytes) {
        approx(size, to_points(5.0 * f64::from(scale), Units::Mm));
    }

    let regular = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let bold = std::fs::read(fixture("fonts/LiberationSerif-Bold.ttf")).unwrap();
    let mut widest = 0.0f32;
    for line in painted_lines(&bytes) {
        let face = if font_program(&bytes, &line.font) == bold {
            liberation_face(&bold)
        } else {
            liberation_face(&regular)
        };
        widest = widest.max(measure_pt(&face, &line.text, line.size));
    }
    assert!(widest <= to_points(70.0, Units::Mm) + 0.01);
    let block = drawn_block_height(&page, &bytes, 6.0 * f64::from(scale));
    assert!(block <= to_points(50.0, Units::Mm) + 0.01);
}

#[test]
fn card_block_is_centered() {
    let (page, _) = load_fixture("card.yaml");
    let bytes = render_card();
    let regular = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let bold = std::fs::read(fixture("fonts/LiberationSerif-Bold.ttf")).unwrap();
    let regular_face = liberation_face(&regular);
    let lines = painted_lines(&bytes);
    let center_x = to_points(45.0, page.page_size.units);
    for line in &lines {
        let face = if font_program(&bytes, &line.font) == bold {
            liberation_face(&bold)
        } else {
            liberation_face(&regular)
        };
        let width = measure_pt(&face, &line.text, line.size);
        let (doc_x, _) = document_point(&page, line.x, line.y);
        approx(doc_x + width / 2.0, center_x);
    }
    let scale = lines[0].size / to_points(5.0, Units::Mm);
    let (_, first_baseline) = document_point(&page, lines[0].x, lines[0].y);
    let ascent =
        lines[0].size * f32::from(regular_face.ascender()) / f32::from(regular_face.units_per_em());
    let block_height = drawn_block_height(&page, &bytes, 6.0 * f64::from(scale));
    approx(
        (first_baseline - ascent) + block_height / 2.0,
        to_points(55.0, page.page_size.units),
    );
}

#[test]
fn card_blank_line() {
    let bytes = render_card();
    let lines = painted_lines(&bytes);
    let regular = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let last_surname = lines
        .iter()
        .rposition(|line| font_program(&bytes, &line.font) == regular)
        .expect("surname line");
    let first_company = lines
        .iter()
        .position(|line| font_program(&bytes, &line.font) != regular)
        .expect("company line");
    let scale = lines[0].size / to_points(5.0, Units::Mm);
    approx(
        (lines[last_surname].y - lines[first_company].y).abs(),
        2.0 * to_points(6.5 * f64::from(scale), Units::Mm),
    );
}

#[test]
fn unknown_tag_writes_nothing() {
    let (page, base_dir) = load_fixture("text-unknown-tag.yaml");
    let out = card_out("unknown-tag.pdf");
    let _ = std::fs::remove_file(&out);
    let err = render_page(&page, &base_dir).expect_err("unknown tag");
    assert!(err.context.contains("line-1"));
    assert!(!out.exists());
}

#[test]
fn leading_tag_changes_step() {
    let (page, base_dir) = load_fixture("text-leading-tag.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let points = baselines(&bytes);
    assert_eq!(points.len(), 2);
    approx((points[0].1 - points[1].1).abs(), to_points(8.0, Units::Mm));
}

#[test]
fn span_font_attribute() {
    let (page, base_dir) = load_fixture("text-span-font.yaml");
    let bytes = render_page(&page, &base_dir).unwrap();
    let used = tj_fonts(&bytes);
    assert_eq!(used.len(), 1);
    let regular = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    assert_eq!(font_program(&bytes, &used[0]), regular);
    assert!(page_text(&bytes).contains("Hi"));
}

#[test]
fn rejects_bare_span_font() {
    let (page, base_dir) = load_fixture("text-span-bare.yaml");
    let out = card_out("span-bare.pdf");
    let _ = std::fs::remove_file(&out);
    let err = render_page(&page, &base_dir).expect_err("bare span font");
    assert!(err.context.contains("line-1"));
    assert!(!out.exists());
}

#[test]
fn rejects_span_font_switch() {
    let (page, base_dir) = load_fixture("text-span-switch.yaml");
    let out = card_out("span-switch.pdf");
    let _ = std::fs::remove_file(&out);
    let err = render_page(&page, &base_dir).expect_err("span closer names a font");
    assert!(err.context.contains("line-1"));
    assert!(!out.exists());
}

#[test]
fn amp_escape() {
    let (page, base_dir) = load_fixture("text-amp.yaml");
    let text = page_text(&render_page(&page, &base_dir).unwrap());
    assert!(text.contains('&'));
    assert!(!text.contains("amp"));
}

#[test]
fn cli_card() {
    let out = card_out("card-cli.pdf");
    let _ = std::fs::remove_file(&out);
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_rust-reg"))
        .arg("render-page")
        .arg(fixture("card.yaml"))
        .arg("-o")
        .arg(&out)
        .status()
        .expect("spawn rust-reg");
    assert!(status.success());
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    let text = page_text(&bytes);
    assert!(text.contains("{{name}}"));
    assert!(text.contains("{{surname}}"));
    assert!(text.contains("{{company name}}"));
    let (dict, count) = page_boxes(&bytes);
    assert_eq!(count, 1);
    let trim = rect_of(&dict, b"TrimBox");
    approx(trim[2] - trim[0], mm(90.0));
    approx(trim[3] - trim[1], mm(130.0));
}

struct PaintedLine {
    text: String,
    font: String,
    size: f32,
    x: f32,
    y: f32,
}

fn painted_lines(bytes: &[u8]) -> Vec<PaintedLine> {
    let texts: Vec<String> = page_text(bytes)
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let mut font = String::new();
    let mut size = 0.0;
    let mut position = None;
    let mut lines = Vec::new();
    for op in content_ops(bytes) {
        if op.operator == "Tf" {
            font = String::from_utf8(op.operands[0].as_name().unwrap().to_vec()).unwrap();
            size = num(&op.operands[1]);
        }
        if op.operator == "Tm" {
            position = Some((num(&op.operands[4]), num(&op.operands[5])));
        }
        if op.operator == "Tj" {
            let (x, y) = position.expect("Tm before Tj");
            lines.push(PaintedLine {
                text: String::new(),
                font: font.clone(),
                size,
                x,
                y,
            });
        }
    }
    assert_eq!(texts.len(), lines.len(), "extracted {texts:?}");
    for (line, text) in lines.iter_mut().zip(texts) {
        line.text = text;
    }
    lines
}

fn tf_sizes(bytes: &[u8]) -> Vec<f32> {
    content_ops(bytes)
        .iter()
        .filter(|op| op.operator == "Tf")
        .map(|op| num(&op.operands[1]))
        .collect()
}

fn drawn_block_height(
    page: &rust_reg::struct_to_pdf::Page,
    bytes: &[u8],
    last_line_height: f64,
) -> f32 {
    let points = baselines(bytes);
    let (_, first) = document_point(page, points[0].0, points[0].1);
    let last = points.last().unwrap();
    let (_, last_y) = document_point(page, last.0, last.1);
    (last_y - first) + to_points(last_line_height, page.page_size.units)
}

fn content_size(page: &rust_reg::struct_to_pdf::Page) -> (f64, f64) {
    let text = text_box(page);
    (
        text.width - text.padding.left - text.padding.right,
        text.height - text.padding.top - text.padding.bottom,
    )
}

#[test]
fn shrinks_wide_word() {
    let (page, base_dir) = load_fixture("text-scale-wide.yaml");
    let text = text_box(&page);
    let (unscaled_width, unscaled_height) = authored_text_bounds(&page, text, &base_dir).unwrap();
    let (content_width, content_height) = content_size(&page);
    assert!(
        unscaled_width > content_width,
        "word width {unscaled_width} already fits"
    );
    assert!(unscaled_height <= content_height);
    let bytes = render_page(&page, &base_dir).unwrap();
    let sizes = tf_sizes(&bytes);
    assert_eq!(sizes.len(), 1);
    let scale = content_width / unscaled_width;
    approx(sizes[0], to_points(text.text.font_size * scale, Units::Mm));
    let font = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font);
    let word = text.text.preentered.as_deref().unwrap();
    approx(
        measure_pt(&face, word, sizes[0]),
        to_points(content_width, Units::Mm),
    );
}

#[test]
fn shrinks_tall_block() {
    let (page, base_dir) = load_fixture("text-scale-tall.yaml");
    let text = text_box(&page);
    let (unscaled_width, unscaled_height) = authored_text_bounds(&page, text, &base_dir).unwrap();
    let (content_width, content_height) = content_size(&page);
    assert!(
        unscaled_height > content_height,
        "block height {unscaled_height} already fits"
    );
    assert!(unscaled_width <= content_width);
    let bytes = render_page(&page, &base_dir).unwrap();
    let scale = content_height / unscaled_height;
    let points = baselines(&bytes);
    let step = to_points(
        (text.text.line_height + text.text.leading) * scale,
        Units::Mm,
    );
    for pair in points.windows(2) {
        approx((pair[0].1 - pair[1].1).abs(), step);
    }
    approx(
        drawn_block_height(&page, &bytes, text.text.line_height * scale),
        to_points(content_height, Units::Mm),
    );
}

#[test]
fn does_not_enlarge() {
    let (page, base_dir) = load_fixture("text-scale-short.yaml");
    let text = text_box(&page);
    let (unscaled_width, unscaled_height) = authored_text_bounds(&page, text, &base_dir).unwrap();
    let (content_width, content_height) = content_size(&page);
    assert!(unscaled_width <= content_width && unscaled_height <= content_height);
    let bytes = render_page(&page, &base_dir).unwrap();
    let sizes = tf_sizes(&bytes);
    assert_eq!(sizes.len(), 1);
    approx(sizes[0], to_points(text.text.font_size, Units::Mm));
}

#[test]
fn off_keeps_authored_size() {
    let (page, base_dir) = load_fixture("text-scale-off.yaml");
    let text = text_box(&page);
    assert!(!text.auto_scale);
    let (unscaled_width, _) = authored_text_bounds(&page, text, &base_dir).unwrap();
    let (content_width, _) = content_size(&page);
    assert!(unscaled_width > content_width);
    let bytes = render_page(&page, &base_dir).unwrap();
    let sizes = tf_sizes(&bytes);
    assert_eq!(sizes.len(), 1);
    approx(sizes[0], to_points(text.text.font_size, Units::Mm));
}

#[test]
fn uniform_span_sizes() {
    let (page, base_dir) = load_fixture("text-scale-span.yaml");
    let text = text_box(&page);
    let (unscaled_width, unscaled_height) = authored_text_bounds(&page, text, &base_dir).unwrap();
    let (content_width, content_height) = content_size(&page);
    assert!(
        unscaled_width > content_width,
        "line width {unscaled_width} already fits"
    );
    let width_scale = content_width / unscaled_width;
    let height_scale = content_height / unscaled_height;
    assert!(width_scale <= height_scale);
    let bytes = render_page(&page, &base_dir).unwrap();
    let mut sizes = tf_sizes(&bytes);
    sizes.sort_by(|left, right| left.partial_cmp(right).unwrap());
    assert_eq!(sizes.len(), 2);
    approx(sizes[0], to_points(5.0 * width_scale, Units::Mm));
    approx(sizes[1], to_points(10.0 * width_scale, Units::Mm));
    approx(sizes[1] / sizes[0], 2.0);
}

#[test]
fn respects_content_box() {
    let (page, base_dir) = load_fixture("text-scale-padding.yaml");
    let text = text_box(&page);
    let (unscaled_width, _) = authored_text_bounds(&page, text, &base_dir).unwrap();
    let (content_width, _) = content_size(&page);
    assert!(unscaled_width > content_width);
    assert!(unscaled_width > text.width);
    let bytes = render_page(&page, &base_dir).unwrap();
    let sizes = tf_sizes(&bytes);
    assert_eq!(sizes.len(), 1);
    let font = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font);
    let word = text.text.preentered.as_deref().unwrap();
    approx(
        measure_pt(&face, word, sizes[0]),
        to_points(content_width, Units::Mm),
    );
}

#[test]
fn empty_with_auto_scale_draws_nothing() {
    let (page, base_dir) = load_fixture("text-scale-empty.yaml");
    assert!(text_box(&page).auto_scale);
    let bytes = render_page(&page, &base_dir).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    assert!(page_text(&bytes).trim().is_empty());
}

#[test]
fn tall_run_stays_inside_after_reflow() {
    let (page, base_dir) = load_fixture("text-scale-reflow.yaml");
    let text = text_box(&page);
    let (unscaled_width, unscaled_height) = authored_text_bounds(&page, text, &base_dir).unwrap();
    let (content_width, content_height) = content_size(&page);
    assert!(
        unscaled_width > content_width,
        "width {unscaled_width} already fits"
    );
    assert!(
        unscaled_height < content_height,
        "unscaled height {unscaled_height} already includes the tall run"
    );
    let bytes = render_page(&page, &base_dir).unwrap();
    let lines = painted_lines(&bytes);
    let font = std::fs::read(fixture("fonts/LiberationSerif-Regular.ttf")).unwrap();
    let face = liberation_face(&font);
    for line in &lines {
        assert!(
            measure_pt(&face, &line.text, line.size) <= to_points(content_width, Units::Mm) + 0.01
        );
    }
    let scale = lines[0].size / to_points(text.text.font_size, Units::Mm);
    let last = lines.last().unwrap();
    let last_line_height = if last.text.split_whitespace().next_back() == Some("i") {
        40.0
    } else {
        text.text.line_height
    };
    let block = drawn_block_height(&page, &bytes, last_line_height * f64::from(scale));
    assert!(
        block <= to_points(content_height, Units::Mm) + 0.01,
        "drawn block {block} exceeds the content box"
    );
}

#[test]
fn one_page_matches_render_page() {
    let (page, base_dir) = load_fixture("card.yaml");
    let single = render_page(&page, &base_dir).unwrap();
    let many = render_pages(std::slice::from_ref(&page), &base_dir).unwrap();
    assert_eq!(pages_count(&many), 1);
    assert_eq!(pages_count(&single), pages_count(&many));
    let text = page_text(&many);
    assert!(text.contains("{{name}}"));
    assert!(text.contains("{{surname}}"));
    assert!(text.contains("{{company name}}"));
    assert_eq!(page_text(&single), text);
}

#[test]
fn two_blank_pages() {
    let first = parse_yaml(
        r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 0, right: 0, bottom: 0, left: 0}
resources:
  images: {}
  fonts: {}
contents: []
"#,
    )
    .unwrap();
    let second = parse_yaml(
        r#"
page_size:
  width: 200
  height: 100
  units: points
  bleeds: {top: 10, right: 10, bottom: 10, left: 10}
resources:
  images: {}
  fonts: {}
contents: []
"#,
    )
    .unwrap();
    let bytes = render_pages(&[first, second], &fixture("")).unwrap();
    assert_eq!(pages_count(&bytes), 2);
    let doc = Document::load_mem(&bytes).unwrap();
    approx_eq(
        rect_of(page_dict(&doc, 1), b"TrimBox"),
        [0.0, 0.0, mm(90.0), mm(110.0)],
    );
    approx_eq(
        rect_of(page_dict(&doc, 2), b"TrimBox"),
        [10.0, 10.0, 210.0, 110.0],
    );
}

#[test]
fn shared_font_object() {
    let (first, base_dir) = regular_page("Ann");
    let (second, _) = regular_page("Bo");
    let bytes = render_pages(&[first, second], &base_dir).unwrap();
    let doc = Document::load_mem(&bytes).unwrap();
    assert_eq!(pages_count(&bytes), 2);
    let file_ids = font_file_ids(&doc);
    assert_eq!(file_ids.len(), 1, "one embedded font file");
    assert_eq!(font_file_on_page(&doc, 1), file_ids[0]);
    assert_eq!(font_file_on_page(&doc, 2), file_ids[0]);
    assert!(doc.extract_text(&[1]).unwrap().contains("Ann"));
    assert!(doc.extract_text(&[2]).unwrap().contains("Bo"));
}

#[test]
fn empty_pages() {
    let err = render_pages(&[], &fixture("")).expect_err("no pages");
    assert_eq!(err.context, "page");
    assert!(err.message.contains("no pages"), "{}", err.message);
}

fn regular_page(text: &str) -> (rust_reg::struct_to_pdf::Page, PathBuf) {
    let source = format!(
        r#"
page_size:
  width: 90
  height: 50
  units: mm
  bleeds: {{top: 0, right: 0, bottom: 0, left: 0}}
resources:
  images: {{}}
  fonts:
    font1:
      name: font1
      font_family: LiberationSerif
      weight: normal
      italic: false
      embedded: true
      source_path: fonts/LiberationSerif-Regular.ttf
contents:
  - id: line
    posX: 10
    posY: 10
    width: 70
    height: 20
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    text:
      font: font1
      font_size: 12
      leading: 0
      line_height: 14
      preentered: "{text}"
      content: null
"#
    );
    (parse_yaml(&source).unwrap(), fixture(""))
}

fn pages_count(bytes: &[u8]) -> i64 {
    let doc = Document::load_mem(bytes).expect("pdf bytes");
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let catalog = doc.get_dictionary(root).unwrap();
    let pages_id = catalog.get(b"Pages").unwrap().as_reference().unwrap();
    let pages = doc.get_dictionary(pages_id).unwrap();
    match pages.get(b"Count").unwrap() {
        Object::Integer(count) => *count,
        other => panic!("Count is {other:?}"),
    }
}

fn page_dict(doc: &Document, number: u32) -> &lopdf::Dictionary {
    let page_id = *doc.get_pages().get(&number).expect("page");
    doc.get_dictionary(page_id).expect("page dict")
}

fn font_file_ids(doc: &Document) -> Vec<lopdf::ObjectId> {
    let mut ids = Vec::new();
    for object in doc.objects.values() {
        let Object::Dictionary(dict) = object else {
            continue;
        };
        if let Ok(Object::Reference(id)) = dict.get(b"FontFile2") {
            ids.push(*id);
        }
    }
    ids
}

fn font_file_on_page(doc: &Document, number: u32) -> lopdf::ObjectId {
    let resources = page_dict(doc, number)
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap();
    let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
    let type0_id = fonts.get(b"F1").unwrap().as_reference().unwrap();
    let type0 = doc.get_dictionary(type0_id).unwrap();
    let descendants = type0.get(b"DescendantFonts").unwrap().as_array().unwrap();
    let cid_id = descendants[0].as_reference().unwrap();
    let cid = doc.get_dictionary(cid_id).unwrap();
    let descriptor_id = cid.get(b"FontDescriptor").unwrap().as_reference().unwrap();
    let descriptor = doc.get_dictionary(descriptor_id).unwrap();
    descriptor
        .get(b"FontFile2")
        .unwrap()
        .as_reference()
        .unwrap()
}

#[test]
fn rejects_bad_auto_scale() {
    let out = card_out("bad-auto-scale.pdf");
    let _ = std::fs::remove_file(&out);
    let err = load_page(&fixture("text-scale-bad.yaml")).expect_err("number is not a boolean");
    assert!(err.context.contains("line-1"), "{}", err.context);
    assert!(!out.exists());
}
