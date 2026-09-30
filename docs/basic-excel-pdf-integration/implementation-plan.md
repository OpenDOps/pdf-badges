# Basic Excel to PDF integration

Build sequence for the first path that turns a workbook into a multi-page PDF. [excel-tooling](../excel-tooling/design.md) is already the reader. The PDF library and service are specified in [pdf-tooling/template/implementation-plan.md](../pdf-tooling/template/implementation-plan.md). This file is the sequence that joins the two, and the Excel caller.

The plan is done when both containers are up and the caller, pointed at `titles.xlsx` and a one-box template, writes a two-page PDF. Page one contains Ann, Oslo, `2`, `2024-03-15`, and `true`. Page two contains Bo, an empty city, `3.5`, `2`, and `true`. Neither page contains `{{`.

Set **Status** to `not started`, `in progress`, or `done`. Status for steps 1 through 3 is tracked in the PDF plan.

## Summary

| Step | Where |
|---|---|
| 1. Extract mustache fields | [Template flag, scan, delimiters, index](../pdf-tooling/template/implementation-plan.md#step-1-template-flag) |
| 2. Fill from those spans | [Fill, multi-page writer, render rows](../pdf-tooling/template/implementation-plan.md#step-5-fill-from-spans), [Rust and C library](../pdf-tooling/template/implementation-plan.md#step-8-embeddable-library) |
| 3. pdf-tooling gRPC | [Proto, LoadTemplate, Render, Docker](../pdf-tooling/template/implementation-plan.md#step-9-proto-and-process) |
| [4. Excel caller](#step-4-excel-caller) | This file. Status: not started |

The caller is a script. It is the only place that knows both the mustache names and the Excel column names.

## Step 4. Excel caller

[Back to summary](#summary)

A Python caller reads the workbook through excel-tooling and posts string rows to pdf-tooling. It owns the pairing of mustache field to column name.

```text
ExcelTooling.OpenWorkbook(file)
ExcelTooling.SelectSheet(...)        # when the sheet is not the first non-empty sheet
ExcelTooling.SelectHeader(session, row)
rows = ExcelTooling.ReadTable(session)

PdfTooling.LoadTemplate(zip)
PdfTooling.Render(session, one ValueRow per excel row)
PdfTooling.Close
ExcelTooling.Close
```

Column names come from `SelectHeader`. Mustache names come from `LoadTemplate`. The binding file pairs them. Names are compared exactly, including case. `{{name}}` is paired with `Name` only when the file says so.

```yaml
fields:
  name: Name
  city: City
  amount: Amount
  when: When
  active: Active
```

The key is the mustache field. The value is `Table.columns[].name`.

### Work

1. Package `modules/basic-integration/`. It depends on `grpcio` and uses the stubs already generated for excel, plus Python stubs generated from `pdf.proto` by the same `scripts/gen_proto.sh` style excel-tooling uses. The caller does not link the Rust crate.

2. Read every `RowBatch` until the stream ends. A failed `ReadTable` stops the caller before `Render`.

3. Format each cell to a string when building a `ValueRow`:

   | `Value` kind | Inserted text |
   |---|---|
   | `text` | the string |
   | `int_value` | decimal, no fraction |
   | `float_value` | shortest round-trip decimal |
   | `bool_value` | `true` or `false` |
   | `date` with `has_time` false | `YYYY-MM-DD` |
   | `date` with `has_time` true | `YYYY-MM-DD HH:MM:SS` |

   A missing cell becomes an empty string. A binding whose column name is not on the table is an error before `Render`. A mustache field returned by `LoadTemplate` and absent from the binding file is an error before `Render`. A binding key that is not a loaded field is an error. Leaf columns only: a duplicated leaf name under two parents is an error. Subcolumn paths wait.

4. One `Render` call carries every data row. The reply is the absolute path of the PDF the service wrote. The caller copies that file to `--output`. `RenderChunks` is the optional byte stream. Zero data rows do not call `Render`.

5. Fixture `tests/fixtures/template/titles.yaml` is one template text box, `template: true`, with `{{name}}`, `{{city}}`, `{{amount}}`, `{{when}}`, and `{{active}}`, using the Liberation fonts already under `tests/fixtures/struct-to-pdf/fonts/`. The smoke zip contains that page and those font files. The binding file is the YAML above.

6. CLI:

   ```text
   python -m basic_integration \
     --excel excel-tooling:50051 \
     --pdf pdf-tooling:50052 \
     --workbook modules/excel-tooling/tests/fixtures/titles.xlsx \
     --sheet Titles \
     --header-row 4 \
     --template tests/fixtures/template/titles.zip \
     --page titles.yaml \
     --binding tests/fixtures/template/titles-binding.yaml \
     --output target/titles.pdf
   ```

   Run the command on the `private` network. Excel is not published to the host. `--sheet` selects by name via `SelectSheet`. `--header-row` is the excel row passed to `SelectHeader`.

### Done when

`docker compose up excel-tooling pdf-tooling` is up, and the command above exits 0.

The PDF has two pages. Extracted text:

| Page | Contains | Also |
|---|---|---|
| 1 | `Ann`, `Oslo`, `2`, `2024-03-15`, `true` | no `{{` |
| 2 | `Bo`, `3.5`, `2`, `true` | no `Oslo`, no `{{` |

Page two has an empty city because row 7 of Titles has no City cell. `When` on that row is the formula result `2`, formatted as an int.

That is the basic integration.

## After this plan

Not required for the smoke test.

- `SetBindings` and `Render` of a typed `Table`, as written in [pdf-tooling/design.md](../pdf-tooling/design.md), including date `format` patterns.
- Streaming rows into `Render` when a sheet exceeds the 64 MiB message.
- Moving the Rust crate from the repository root to `modules/pdf-tooling/`.
