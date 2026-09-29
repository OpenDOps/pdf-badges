# Current design

`rust-reg` is a Rust CLI that reads one PDF and writes a structured description of its pages. The description is the input the later editor and PDF writer are meant to share. This document describes the program as it works today.

```bash
rust-reg input.pdf [-o output]
```

`output` defaults to `output/`. The command writes `analysis.json` and `analysis.yaml` there. Image bytes are written to `extracted/` in the working directory, ignoring `-o`.

Package `rust-reg` 0.1.0, edition 2021. Dependencies: `lopdf` 0.38, `clap` 4.4, `flate2`, `serde`, `serde_json`, `serde_yaml`. There is no test suite.

## Pipeline

`src/main.rs` parses arguments, creates the output directory, calls `StructuredPdfAnalyzer::analyze_pdf`, prints page, image, and font counts, then saves JSON and YAML.

`src/modules/structured_analyzer.rs` does the work:

1. Load the file with `lopdf::Document::load_from`.
2. Read document metadata from the trailer `Info` dictionary.
3. For each page from `doc.get_pages()`:
   - page size from `MediaBox`, scaled by `UserUnit` when present (fallback 595.92 × 842.04 points). `TrimBox` and `BleedBox` are not read, so the file has no `bleeds` field
   - images and fonts from the page `Resources` dictionary
   - content boxes from the page `Contents` stream or array of streams
4. Serialize `PdfAnalysis` (`src/modules/data_structures.rs`).

Each content stream is inflated when its filter is the name `FlateDecode`, then parsed with `lopdf::content::Content::decode`. Text and images are collected in two separate passes over the same operators.

### Text pass

| Operator | Effect |
|---|---|
| `Tm` | Starts a new line and stores x (`operands[4]`) and y (`operands[5]`) |
| `Tf` | Stores the font resource name and size |
| `Tj`, `TJ` | Appends string bytes, including strings nested in a `TJ` array |
| `ET` | Flushes the current line |

`BT`, `Td`, and `TD` are ignored. Every emitted line reuses the last `Tm` x. Width is `character_count * font_size * 0.6`. Line height is the average gap between recorded Y positions. Alignment compares margins to a fixed page width of 595.92 points, and the positions used for that comparison are the final x and y only.

Decoded text comes from a hardcoded CID-to-Unicode table copied from one document (a handful of Cyrillic mappings and four ranges). The font’s real `ToUnicode` stream is stored on the font resource and is not used for decoding. `FontProperties` is always the default: not italic, not bold, not underlined, no family.

### Image pass

A viewport matrix `[1, 0, 0, -1, 0, pageHeight]` flips PDF’s bottom-left origin. Each `cm` concatenates a six-number transform onto that matrix (MuPDF `fz_concat` order). `Do` looks up the XObject name in the page image map and, on a hit, emits an image box:

- `posX` and `posY` from the matrix translation
- `posY_Pdf` as `pageHeight - posY`
- `width` and `height` from the absolute scale components
- the full six-number matrix on the image object

After every `Do`, the running matrix is reset to the viewport. `q` and `Q` are not implemented, so save/restore and nested transforms are lost.

Image files are written while resources are collected, before content operators run:

| Filter | File |
|---|---|
| `DCTDecode` | `.jpg` raw stream bytes |
| `JPXDecode` | `.jp2` raw stream bytes |
| `FlateDecode` | inflate, then a BMP header from `utils.rs` |
| anything else, or a filter array | treated as BMP |

Only a filter stored as a single name `FlateDecode` is inflated. A filter array, or a second filter after inflate, is left compressed.

## Data model

Types live in `src/modules/data_structures.rs`. Field-level rules are in [document-schema.md](document-schema.md).

```text
PdfAnalysis
  metadata          title, author, creator, producer, dates, page_count
  pages[]
    page_number, page_id
    page_size       width, height, units, media_box (no bleeds)
    resources
      images{}      pixel size, color space, filter, extracted path
      fonts{}       base font, subtype, encoding, raw ToUnicode
    contents[]      untagged text box or image box
```

`ContentBox` is `#[serde(untagged)]`. A text object has `text` and no `image`. An image object has `image` and `posY_Pdf`. Text `posY` is the PDF text-matrix Y (origin at the bottom). Image `posY` is the raw translation; `posY_Pdf` is flipped. The two kinds of box do not share one coordinate system.

A checked-in run of `print.pdf` (Chromium / Skia, one page) produced two text boxes and three image boxes, font `F6` (`AAAAAA+LiberationSerif-Bold`, `Identity-H`), and a media box of 595.92 × 842.88 points.

## Module split

The print-only analyzers were test scaffolding. They are gone. One implementation remains, the functions that used to live in `structured_analyzer.rs`, now split by job. `structured_analyzer` opens the PDF, calls the three modules, assembles `PdfAnalysis`, and writes JSON and YAML.

| File | Owns |
|---|---|
| `pdf_analyzer.rs` | Metadata, page size, the `Resources` walk, inflating a content stream, `Content::decode` |
| `text_analyzer.rs` | Text operators to `ContentBox` values, the CID decode |
| `image_extractor.rs` | Image resources, filter and file format, the file write, BMP via `utils.rs`, `cm` / `Do` placement |

## Known limitations

Work is grouped in [inventarization-plan.md](inventarization-plan.md). The build sequence, including font files, is [extractor/plan.md](extractor/plan.md).

**Text processing**

1. **Decoding is document-specific.** Characters outside the hardcoded map are dropped, including glyphs whose real `ToUnicode` CMap is already sitting on the font resource.
2. **Text geometry is approximate.** One x for every line, ignored `Td`/`TD`, estimated width, and alignment against a fixed A4 width.
3. **Style is unused.** Bold, italic, underline, and family are never filled, even when the base font name contains them (the sample font is `LiberationSerif-Bold` and still reports `is_bold: false`).

**Image processing**

4. **Graphics state is flat.** No `q`/`Q` stack. The image matrix resets after each `Do`. Form XObjects and resources inherited from a parent `Pages` node are not walked. Some sample image boxes land far outside the page.
5. **Only a single `FlateDecode` name is inflated.** Filter arrays and filter chains are not decoded. Color spaces such as `ICCBased` and `DeviceCMYK` are labeled and then written as if they were RGB BMP.

**Test coverage**

6. **There are no tests.** Decode, geometry, image placement, and serialization have no fixture or golden file.

Operators listed as TODOs at the top of `structured_analyzer.rs` and not read on the live path: `q`, `Q`, `gs`, `re`, `W*`, `n`, `RG`, `rg`, `BT`, `Td`, `TD`.
