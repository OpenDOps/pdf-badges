use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::process::Command;

use std::collections::HashMap;

use lopdf::Document;

use rust_reg::struct_to_pdf::ffi::{
    pdf_template_bytes_free, pdf_template_field_count, pdf_template_field_name, pdf_template_free,
    pdf_template_index_bytes, pdf_template_last_error, pdf_template_render, PdfTemplate,
    PdfTemplateValue,
};
use rust_reg::struct_to_pdf::{
    fill, index_template, load_page, parse_page, render_rows, save_pdf, scan, to_points,
    ContentEntry, IndexedBox, RenderError, SourceFormat, TemplateIndex, Units,
};

fn parse_yaml(source: &str) -> Result<rust_reg::struct_to_pdf::Page, RenderError> {
    parse_page(source, SourceFormat::Yaml)
}

fn text_page(template_line: &str) -> String {
    format!(
        r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {{top: 3, right: 3, bottom: 3, left: 3}}
resources:
  images: {{}}
  fonts: {{}}
contents:
  - id: box-1
    posX: 10
    posY: 30
    width: 70
    height: 50
    {template_line}
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    text:
      font: font1
      font_size: 5
      leading: 0.5
      line_height: 6
      preentered: hello
      content: null
"#
    )
}

fn text_box(page: &rust_reg::struct_to_pdf::Page) -> &rust_reg::struct_to_pdf::TextBox {
    match &page.contents[0] {
        ContentEntry::Text(text) => text,
        _ => panic!("expected a text box"),
    }
}

fn assert_scan_message(source: &str, needle: &str) {
    let err = scan(source, "{{", "}}", "box-1").expect_err("expected a scan error");
    assert_eq!(err.context, "box-1");
    assert!(
        err.message.contains(needle),
        "message {:?} does not contain {needle}",
        err.message
    );
}

#[test]
fn template_defaults_false() {
    let page = parse_yaml(&text_page("")).unwrap();
    assert!(!text_box(&page).template);
}

#[test]
fn template_true() {
    let page = parse_yaml(&text_page("template: true")).unwrap();
    assert!(text_box(&page).template);
}

#[test]
fn template_not_a_bool() {
    let err = parse_yaml(&text_page("template: yes")).expect_err("yes is not a bool");
    assert_eq!(err.context, "box-1");
    assert!(
        err.message.contains("template"),
        "message {:?} does not contain template",
        err.message
    );
}

#[test]
fn template_on_image() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 3, right: 3, bottom: 3, left: 3}
resources:
  images: {}
  fonts: {}
contents:
  - id: photo-1
    posX: 10
    posY: 20
    width: 30
    height: 40
    template: true
    image:
      resource_name: photo
"#;
    let err = parse_yaml(source).expect_err("template on an image");
    assert_eq!(err.context, "photo-1");
    assert!(
        err.message.contains("template"),
        "message {:?} does not contain template",
        err.message
    );
}

#[test]
fn card_yaml_unchanged() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/struct-to-pdf/card.yaml");
    let page = load_page(&path).expect("card yaml");
    assert!(!text_box(&page).template);
}

#[test]
fn spans_cover_braces() {
    let source = "Hello {{name}}";
    let holes = scan(source, "{{", "}}", "box-1").unwrap();
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].name, "name");
    assert_eq!(&source[holes[0].from..holes[0].to], "{{name}}");
    assert_eq!(&source[..holes[0].from], "Hello ");
}

#[test]
fn trim_inside() {
    let source = "{{ name }}";
    let holes = scan(source, "{{", "}}", "box-1").unwrap();
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].name, "name");
    assert_eq!(&source[holes[0].from..holes[0].to], "{{ name }}");
}

#[test]
fn inner_space_stays() {
    let holes = scan("{{company name}}", "{{", "}}", "box-1").unwrap();
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].name, "company name");
}

#[test]
fn repeated_name_two_spans() {
    let source = "{{name}} and {{name}}";
    let holes = scan(source, "{{", "}}", "box-1").unwrap();
    assert_eq!(holes.len(), 2);
    assert_eq!(holes[0].name, "name");
    assert_eq!(holes[1].name, "name");
    assert!(holes[0].from < holes[1].from);
    assert_eq!(&source[holes[0].from..holes[0].to], "{{name}}");
    assert_eq!(&source[holes[1].from..holes[1].to], "{{name}}");
}

#[test]
fn literal_brace() {
    let holes = scan("a {b} c", "{{", "}}", "box-1").unwrap();
    assert!(holes.is_empty());
}

#[test]
fn empty_braces() {
    assert_scan_message("{{}}", "empty");
}

#[test]
fn nested() {
    assert_scan_message("{{na{{me}}", "nested");
}

#[test]
fn unclosed() {
    assert_scan_message("{{name", "unclosed");
}

#[test]
fn section_tag() {
    assert_scan_message("{{#row}}", "unsupported");
}

#[test]
fn card_string() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/struct-to-pdf/card.yaml");
    let page = load_page(&path).expect("card yaml");
    let source = text_box(&page)
        .text
        .preentered
        .as_deref()
        .expect("preentered");
    let holes = scan(source, "{{", "}}", "card-text").unwrap();
    let mut names = Vec::new();
    for hole in &holes {
        if !names.contains(&hole.name.as_str()) {
            names.push(hole.name.as_str());
        }
    }
    assert_eq!(names, ["name", "surname", "company name"]);
}

#[test]
fn delimiters_default() {
    let page = parse_yaml(&text_page("template: true")).unwrap();
    let text = text_box(&page);
    assert!(text.template);
    assert_eq!(text.delimiter_open, "{{");
    assert_eq!(text.delimiter_close, "}}");
}

#[test]
fn triple_braces() {
    let source = "{{name}} {{{name}}}";
    let holes = scan(source, "{{{", "}}}", "box-1").unwrap();
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].name, "name");
    assert_eq!(&source[holes[0].from..holes[0].to], "{{{name}}}");
    assert!(source[..holes[0].from].contains("{{name}}"));
}

#[test]
fn percent_pair() {
    let source = "{{name}} <%name%>";
    let holes = scan(source, "<%", "%>", "box-1").unwrap();
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].name, "name");
    assert_eq!(&source[holes[0].from..holes[0].to], "<%name%>");
    assert!(source[..holes[0].from].contains("{{name}}"));
}

#[test]
fn delimiters_without_template() {
    let err = parse_yaml(&text_page(
        "delimiters:\n      open: \"{{\"\n      close: \"}}\"",
    ))
    .expect_err("delimiters without template");
    assert_eq!(err.context, "box-1");
    assert!(
        err.message.contains("delimiters"),
        "message {:?} does not contain delimiters",
        err.message
    );
}

#[test]
fn empty_open() {
    let err = parse_yaml(&text_page(
        "template: true\n    delimiters:\n      open: \"\"\n      close: \"}}\"",
    ))
    .expect_err("empty open");
    assert_eq!(err.context, "box-1");
    assert!(
        err.message.contains("empty"),
        "message {:?} does not contain empty",
        err.message
    );
}

#[test]
fn missing_close() {
    let err = parse_yaml(&text_page(
        "template: true\n    delimiters:\n      open: \"{{\"",
    ))
    .expect_err("missing close");
    assert_eq!(err.context, "box-1");
    assert!(
        err.message.contains("delimiters"),
        "message {:?} does not contain delimiters",
        err.message
    );
}

#[test]
fn delimiters_on_image() {
    let source = r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {top: 3, right: 3, bottom: 3, left: 3}
resources:
  images: {}
  fonts: {}
contents:
  - id: photo-1
    posX: 10
    posY: 20
    width: 30
    height: 40
    delimiters:
      open: "{{"
      close: "}}"
    image:
      resource_name: photo
"#;
    let err = parse_yaml(source).expect_err("delimiters on an image");
    assert_eq!(err.context, "photo-1");
    assert!(
        err.message.contains("delimiters"),
        "message {:?} does not contain delimiters",
        err.message
    );
}

fn page_yaml(contents: &str) -> String {
    format!(
        r#"
page_size:
  width: 90
  height: 110
  units: mm
  bleeds: {{top: 3, right: 3, bottom: 3, left: 3}}
resources:
  images: {{}}
  fonts: {{}}
contents:
{contents}
"#
    )
}

fn text_entry(id: &str, flags: &str, preentered: Option<&str>, content: Option<&str>) -> String {
    let preentered = match preentered {
        Some(text) => format!("preentered: \"{text}\""),
        None => String::new(),
    };
    let content = match content {
        Some(text) => format!("content: \"{text}\""),
        None => String::new(),
    };
    format!(
        r#"
  - id: {id}
    posX: 10
    posY: 30
    width: 70
    height: 50
    {flags}
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    text:
      font: font1
      font_size: 5
      leading: 0.5
      line_height: 6
      {preentered}
      {content}
"#
    )
}

fn index_contents(contents: &str) -> Result<TemplateIndex, RenderError> {
    let page = parse_yaml(&page_yaml(contents))?;
    index_template(&page)
}

fn field_names(index: &TemplateIndex) -> Vec<&str> {
    index
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect()
}

#[test]
fn card_fields() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/struct-to-pdf/card.yaml");
    let mut page = load_page(&path).expect("card yaml");
    match &mut page.contents[0] {
        ContentEntry::Text(text) => text.template = true,
        _ => panic!("expected a text box"),
    }
    let index = index_template(&page).unwrap();
    assert_eq!(field_names(&index), ["name", "surname", "company name"]);
    for field in &index.fields {
        assert_eq!(field.box_ids, ["card-text"]);
    }
}

#[test]
fn two_boxes() {
    let contents = format!(
        "{}{}",
        text_entry("a", "template: true", Some("{{name}}"), None),
        text_entry("b", "template: true", Some("{{name}} {{city}}"), None),
    );
    let index = index_contents(&contents).unwrap();
    assert_eq!(field_names(&index), ["name", "city"]);
    assert_eq!(index.fields[0].box_ids, ["a", "b"]);
    assert_eq!(index.fields[1].box_ids, ["b"]);
}

#[test]
fn box_id_once() {
    let contents = text_entry(
        "box-1",
        "template: true",
        Some("{{name}} and {{name}}"),
        None,
    );
    let index = index_contents(&contents).unwrap();
    assert_eq!(field_names(&index), ["name"]);
    assert_eq!(index.fields[0].box_ids, ["box-1"]);
    assert_eq!(index.boxes[0].spans("name"), vec![(0, 8), (13, 21)]);
}

#[test]
fn skips_unmarked_box() {
    let contents = format!(
        "{}{}",
        text_entry("plain", "", Some("{{name}}"), None),
        text_entry("marked", "template: true", Some("{{city}}"), None),
    );
    let index = index_contents(&contents).unwrap();
    assert_eq!(field_names(&index), ["city"]);
    assert_eq!(index.boxes.len(), 1);
    assert_eq!(index.boxes[0].box_id, "marked");
}

#[test]
fn content_wins() {
    let contents = text_entry(
        "box-1",
        "template: true",
        Some("{{name}}"),
        Some("{{city}}"),
    );
    let index = index_contents(&contents).unwrap();
    assert_eq!(field_names(&index), ["city"]);
    assert_eq!(index.boxes[0].source, "{{city}}");
}

#[test]
fn empty_braces_names_the_box() {
    let contents = text_entry("card-text", "template: true", Some("{{}}"), None);
    let err = index_contents(&contents).expect_err("empty braces");
    assert_eq!(err.context, "card-text");
}

#[test]
fn no_template_boxes() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/struct-to-pdf/card.yaml");
    let page = load_page(&path).expect("card yaml");
    let index = index_template(&page).unwrap();
    assert!(index.fields.is_empty());
    assert!(index.boxes.is_empty());
}

#[test]
fn missing_text() {
    let contents = text_entry("box-1", "template: true", None, None);
    let err = index_contents(&contents).expect_err("missing text");
    assert_eq!(err.context, "box-1");
    assert!(
        err.message
            .contains("preentered and content are both missing"),
        "message {:?}",
        err.message
    );
}

#[test]
fn two_pairs() {
    let city = text_entry(
        "b",
        "template: true\n    delimiters:\n      open: \"<%\"\n      close: \"%>\"",
        Some("<%city%>"),
        None,
    );
    let contents = format!(
        "{}{}",
        text_entry("a", "template: true", Some("{{name}}"), None),
        city,
    );
    let index = index_contents(&contents).unwrap();
    assert_eq!(field_names(&index), ["name", "city"]);
    assert_eq!(index.fields[0].box_ids, ["a"]);
    assert_eq!(index.fields[1].box_ids, ["b"]);
}

fn indexed(source: &str) -> IndexedBox {
    IndexedBox {
        box_id: "box-1".to_string(),
        holes: scan(source, "{{", "}}", "box-1").unwrap(),
        source: source.to_string(),
    }
}

fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

fn reserved(indexed: &IndexedBox, values: &HashMap<String, String>) -> usize {
    let mut capacity = indexed.source.len();
    for hole in &indexed.holes {
        capacity -= hole.to - hole.from;
        if let Some(value) = values.get(&hole.name) {
            capacity += value
                .chars()
                .map(|ch| match ch {
                    '&' => 5,
                    '<' => 4,
                    '>' => 4,
                    _ => ch.len_utf8(),
                })
                .sum::<usize>();
        }
    }
    capacity
}

#[test]
fn fill_is_one_pass() {
    let source = "{{name}}<br>{{surname}}";
    let indexed = indexed(source);
    let before = indexed.spans("name");
    let filled = fill(&indexed, &values(&[("name", "Ann"), ("surname", "Lee")]));
    assert_eq!(filled, "Ann<br>Lee");
    assert!(!filled.contains("{{"));
    assert_eq!(indexed.spans("name"), before);
    assert_eq!(
        &indexed.source[indexed.holes[0].from..indexed.holes[0].to],
        "{{name}}"
    );
    assert_eq!(
        &indexed.source[indexed.holes[1].from..indexed.holes[1].to],
        "{{surname}}"
    );
}

#[test]
fn repeated_name_same_value() {
    let indexed = indexed("{{name}} and {{name}}");
    let filled = fill(&indexed, &values(&[("name", "Ann")]));
    assert_eq!(filled, "Ann and Ann");
}

#[test]
fn value_with_braces_stays() {
    let source = "Hello {{name}}";
    let indexed = indexed(source);
    let holes_before = indexed.holes.clone();
    let filled = fill(&indexed, &values(&[("name", "{{nope}}")]));
    assert_eq!(filled, "Hello {{nope}}");
    assert_eq!(scan(source, "{{", "}}", "box-1").unwrap(), holes_before);
    assert_eq!(indexed.holes, holes_before);
}

#[test]
fn escapes_markup() {
    let source = "<span>{{name}}</span>";
    let indexed = indexed(source);
    let filled = fill(&indexed, &values(&[("name", "A & B <C>")]));
    assert_eq!(filled, "<span>A &amp; B &lt;C&gt;</span>");
    assert!(filled.starts_with("<span>"));
    assert!(filled.ends_with("</span>"));
}

#[test]
fn missing_value_is_empty() {
    let indexed = indexed("{{name}}<br>{{surname}}");
    let filled = fill(&indexed, &values(&[("name", "Ann")]));
    assert_eq!(filled, "Ann<br>");
    assert!(!filled.contains("surname"));
    assert!(!filled.contains("{{"));
}

#[test]
fn two_pages() {
    let (page, base_dir) = template_card();
    let index = index_template(&page).unwrap();
    let rows = sample_rows();
    let bytes = render_rows(&page, &index, &rows, &base_dir).unwrap();
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/template-pages.pdf");
    save_pdf(&bytes, &path).unwrap();
    let stored = std::fs::read(&path).unwrap();
    assert_eq!(stored, bytes);
    assert!(stored.starts_with(b"%PDF"));
    let doc = Document::load_mem(&stored).unwrap();
    assert_eq!(doc.get_pages().len(), rows.len());
    let expected = [
        ("Ann", "Lee", "North"),
        ("Bo", "Kim", "South"),
        ("Ida", "Berg", "Oslo"),
        (
            "Alexandria",
            "Bartholomew-Worthington",
            "International Cartographic Society",
        ),
        (
            "Maximilian",
            "Supercalifragilisticexpialidocious",
            "Western River Valley",
        ),
        ("Chen", "Wu", "East"),
    ];
    for (index, (name, surname, company)) in expected.iter().enumerate() {
        let text = doc
            .extract_text(&[index as u32 + 1])
            .unwrap()
            .replace('\n', " ");
        assert!(
            text.contains(name),
            "page {} missing {name}: {text}",
            index + 1
        );
        assert!(
            text.contains(surname),
            "page {} missing {surname}: {text}",
            index + 1
        );
        assert!(
            text.contains(company),
            "page {} missing {company}: {text}",
            index + 1
        );
        assert!(
            !text.contains("{{"),
            "page {} still has braces: {text}",
            index + 1
        );
    }
    assert!(!doc.extract_text(&[2]).unwrap().contains("Ann"));
    let authored = to_points(12.0, Units::Mm);
    let short = max_font_size(&doc, 1);
    let long = max_font_size(&doc, 5);
    let closing = max_font_size(&doc, 6);
    assert!(
        (short - authored).abs() < 0.05,
        "short page drew {short}, authored size is {authored}"
    );
    assert!(
        (closing - authored).abs() < 0.05,
        "last short page drew {closing}, authored size is {authored}"
    );
    assert!(
        long < short - 1.0,
        "long name drew {long}, short name drew {short}"
    );
}

#[test]
fn unmarked_box_unchanged() {
    let (mut page, base_dir) = template_card();
    let ContentEntry::Text(template) = &page.contents[0] else {
        panic!("card entry is a text box");
    };
    let mut literal = template.clone();
    literal.id = "literal".to_string();
    literal.template = false;
    literal.pos_y = 28.0;
    literal.text.preentered = Some("{{name}}".to_string());
    literal.text.content = None;
    page.contents.push(ContentEntry::Text(literal));
    let index = index_template(&page).unwrap();
    let bytes = render_rows(
        &page,
        &index,
        &[row("Ann", "Lee", "North"), row("Bo", "Kim", "South")],
        &base_dir,
    )
    .unwrap();
    let doc = Document::load_mem(&bytes).unwrap();
    let first = doc.extract_text(&[1]).unwrap();
    let second = doc.extract_text(&[2]).unwrap();
    assert!(first.contains("{{name}}"));
    assert!(first.contains("Ann"));
    assert!(second.contains("{{name}}"));
    assert!(second.contains("Bo"));
}

#[test]
fn unknown_field_errors() {
    let (page, base_dir) = template_card();
    let index = index_template(&page).unwrap();
    let mut bad = row("Ann", "Lee", "North");
    bad.insert("nope".to_string(), "x".to_string());
    let err = render_rows(&page, &index, &[bad], &base_dir).expect_err("unknown field");
    assert_eq!(err.context, "page");
    assert!(err.message.contains("nope"), "{}", err.message);
}

#[test]
fn render_missing_value_is_empty() {
    let (page, base_dir) = template_card();
    let index = index_template(&page).unwrap();
    let bytes = render_rows(
        &page,
        &index,
        &[values(&[("name", "Ann"), ("company name", "North")])],
        &base_dir,
    )
    .unwrap();
    let text = Document::load_mem(&bytes)
        .unwrap()
        .extract_text(&[1])
        .unwrap();
    assert!(text.contains("Ann"));
    assert!(text.contains("North"));
    assert!(!text.contains("{{surname}}"), "{text}");
}

#[test]
fn zero_rows() {
    let (page, base_dir) = template_card();
    let index = index_template(&page).unwrap();
    let err = render_rows(&page, &index, &[], &base_dir).expect_err("no rows");
    assert_eq!(err.context, "page");
    assert!(err.message.contains("no rows"), "{}", err.message);
}

#[test]
fn holes_survive_render() {
    let (page, base_dir) = template_card();
    let index = index_template(&page).unwrap();
    let before = index.boxes[0].holes.clone();
    render_rows(
        &page,
        &index,
        &[row("Ann", "Lee", "North"), row("Bo", "Kim", "South")],
        &base_dir,
    )
    .unwrap();
    assert_eq!(index.boxes[0].holes, before);
    for hole in &index.boxes[0].holes {
        let slice = &index.boxes[0].source[hole.from..hole.to];
        assert_eq!(slice, format!("{{{{{}}}}}", hole.name));
    }
}

fn template_card() -> (rust_reg::struct_to_pdf::Page, PathBuf) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/template/card.yaml");
    let page = load_page(&path).unwrap_or_else(|err| panic!("load template card: {err}"));
    let base_dir = path.parent().expect("fixture directory").to_path_buf();
    (page, base_dir)
}

fn row(name: &str, surname: &str, company: &str) -> HashMap<String, String> {
    values(&[
        ("name", name),
        ("surname", surname),
        ("company name", company),
    ])
}

fn sample_rows() -> Vec<HashMap<String, String>> {
    vec![
        row("Ann", "Lee", "North"),
        row("Bo", "Kim", "South"),
        row("Ida", "Berg", "Oslo"),
        row(
            "Alexandria",
            "Bartholomew-Worthington",
            "International Cartographic Society of the Northern Territories",
        ),
        row(
            "Maximilian",
            "Supercalifragilisticexpialidocious",
            "Consolidated Agricultural Cooperative of the Western River Valley",
        ),
        row("Chen", "Wu", "East"),
    ]
}

fn max_font_size(doc: &Document, page_number: u32) -> f32 {
    let page_id = *doc.get_pages().get(&page_number).unwrap();
    doc.get_and_decode_page_content(page_id)
        .unwrap()
        .operations
        .iter()
        .filter(|op| op.operator == "Tf")
        .map(|op| match &op.operands[1] {
            lopdf::Object::Real(value) => *value,
            lopdf::Object::Integer(value) => *value as f32,
            other => panic!("Tf size {other:?}"),
        })
        .fold(0.0, f32::max)
}

#[test]
fn rust_crate_renders_rows() {
    let (page, base_dir) = template_card();
    let index = index_template(&page).unwrap();
    let bytes = render_rows(
        &page,
        &index,
        &[row("Ann", "Lee", "North"), row("Bo", "Kim", "South")],
        &base_dir,
    )
    .unwrap();
    let doc = Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.get_pages().len(), 2);
    assert!(doc.extract_text(&[1]).unwrap().contains("Ann"));
    assert!(doc.extract_text(&[2]).unwrap().contains("Bo"));
}

#[test]
fn c_abi_two_pages() {
    let (page, base_dir) = template_card();
    let index = index_template(&page).unwrap();
    let rows = [row("Ann", "Lee", "North"), row("Bo", "Kim", "South")];
    let rust_bytes = render_rows(&page, &index, &rows, &base_dir).unwrap();
    let template = index_card();
    let names = c_field_names(template);
    assert_eq!(names, ["name", "surname", "company name"]);
    let c_bytes = c_render(template, &rows);
    pdf_template_free(template);
    let rust_doc = Document::load_mem(&rust_bytes).unwrap();
    let c_doc = Document::load_mem(&c_bytes).unwrap();
    assert_eq!(c_doc.get_pages().len(), 2);
    assert_eq!(
        c_doc.extract_text(&[1]).unwrap(),
        rust_doc.extract_text(&[1]).unwrap()
    );
    assert_eq!(
        c_doc.extract_text(&[2]).unwrap(),
        rust_doc.extract_text(&[2]).unwrap()
    );
}

#[test]
fn c_abi_unknown_field() {
    let template = index_card();
    let mut bad = row("Ann", "Lee", "North");
    bad.insert("nope".to_string(), "x".to_string());
    let mut out = std::ptr::null_mut();
    let mut len = 0usize;
    let (values, _owned, lengths) = c_values(&[bad]);
    let code = pdf_template_render(
        template,
        values.as_ptr(),
        lengths.as_ptr(),
        1,
        &mut out,
        &mut len,
    );
    pdf_template_free(template);
    assert_ne!(code, 0);
    assert!(out.is_null());
    let message = unsafe { CStr::from_ptr(pdf_template_last_error()) };
    assert!(message.to_str().unwrap().contains("nope"), "{message:?}");
}

#[test]
fn c_abi_zero_rows() {
    let template = index_card();
    let mut out = std::ptr::null_mut();
    let mut len = 0usize;
    let code = pdf_template_render(
        template,
        std::ptr::null(),
        std::ptr::null(),
        0,
        &mut out,
        &mut len,
    );
    pdf_template_free(template);
    assert_ne!(code, 0);
    assert!(out.is_null());
    let message = unsafe { CStr::from_ptr(pdf_template_last_error()) };
    assert!(message.to_str().unwrap().contains("no rows"), "{message:?}");
}

#[test]
fn python_embed_writes_pdf() {
    let library = shared_library();
    assert!(library.is_file(), "missing {}", library.display());
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("modules/pdf-tooling/tests/test_embed.py");
    let status = Command::new("python3")
        .arg(&script)
        .env("PDF_TEMPLATE_LIB", &library)
        .status()
        .expect("python3");
    assert!(status.success());
    let pdf =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/struct-to-pdf/template-python.pdf");
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    let doc = Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.get_pages().len(), 2);
    let first = doc.extract_text(&[1]).unwrap();
    let second = doc.extract_text(&[2]).unwrap();
    assert!(first.contains("Ann") && first.contains("Lee") && first.contains("North"));
    assert!(second.contains("Bo") && second.contains("Kim") && second.contains("South"));
    assert!(!second.contains("Ann"));
}

fn index_card() -> *mut PdfTemplate {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/template/card.yaml");
    let yaml = std::fs::read(&path).unwrap();
    let base = CString::new(path.parent().unwrap().to_str().unwrap()).unwrap();
    let mut out = std::ptr::null_mut();
    let code = pdf_template_index_bytes(yaml.as_ptr(), yaml.len(), base.as_ptr(), &mut out);
    assert_eq!(code, 0, "{:?}", unsafe {
        CStr::from_ptr(pdf_template_last_error())
    });
    out
}

fn c_field_names(template: *mut PdfTemplate) -> Vec<String> {
    let count = pdf_template_field_count(template);
    (0..count)
        .map(|index| {
            let name = unsafe { CStr::from_ptr(pdf_template_field_name(template, index)) };
            name.to_str().unwrap().to_string()
        })
        .collect()
}

fn c_render(template: *mut PdfTemplate, rows: &[HashMap<String, String>]) -> Vec<u8> {
    let (values, _owned, lengths) = c_values(rows);
    let mut out = std::ptr::null_mut();
    let mut len = 0usize;
    let code = pdf_template_render(
        template,
        values.as_ptr(),
        lengths.as_ptr(),
        rows.len(),
        &mut out,
        &mut len,
    );
    assert_eq!(code, 0, "{:?}", unsafe {
        CStr::from_ptr(pdf_template_last_error())
    });
    let bytes = unsafe { std::slice::from_raw_parts(out, len) }.to_vec();
    pdf_template_bytes_free(out, len);
    bytes
}

fn c_values(rows: &[HashMap<String, String>]) -> (Vec<PdfTemplateValue>, Vec<CString>, Vec<usize>) {
    let mut owned = Vec::new();
    let mut values = Vec::new();
    let mut lengths = Vec::new();
    for row in rows {
        lengths.push(row.len());
        for (field, value) in row {
            let field = CString::new(field.as_str()).unwrap();
            let value = CString::new(value.as_str()).unwrap();
            values.push(PdfTemplateValue {
                field: field.as_ptr(),
                value: value.as_ptr(),
            });
            owned.push(field);
            owned.push(value);
        }
    }
    (values, owned, lengths)
}

fn shared_library() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .args(["build", "--lib"])
        .status()
        .expect("cargo build --lib");
    assert!(status.success());
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let filename = if cfg!(target_os = "macos") {
        "librust_reg.dylib"
    } else if cfg!(target_os = "windows") {
        "rust_reg.dll"
    } else {
        "librust_reg.so"
    };
    target.join(profile).join(filename)
}

#[test]
fn capacity_holds() {
    let indexed = indexed("Hello {{name}}");
    let values = values(&[("name", "A & B")]);
    let filled = fill(&indexed, &values);
    assert_eq!(filled, "Hello A &amp; B");
    assert_eq!(filled.len(), reserved(&indexed, &values));
}
