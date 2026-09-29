# Table contract

`proto/irbis/table/v1/table.proto` is the only type both services share. `excel-tooling` produces a `Table`. `pdf-tooling` consumes a `Table`. Column names in the table are the names the PDF binding uses.

A protobuf `map` does not keep order, so columns are a repeated field, left to right, in sheet order.

```protobuf
syntax = "proto3";
package irbis.table.v1;

message Table {
  repeated Column columns = 1;
  repeated Row rows = 2;
}

message Column {
  string excel_id = 1;   // "A", "B", "AA", as shown in the sheet UI
  string name = 2;       // header cell text, trimmed
  repeated Column subcolumns = 3;
}

message Row {
  uint32 excel_row = 1;  // 1-based sheet row the values came from
  repeated Cell cells = 2;
}

message Cell {
  string excel_id = 1;
  Value value = 2;
}

message Value {
  oneof kind {
    string text = 1;
    double float_value = 2;
    int64 int_value = 3;
    DateTimeValue date = 4;
    bool bool_value = 5;
  }
}

message DateTimeValue {
  int32 year = 1;
  int32 month = 2;     // 1-12
  int32 day = 3;       // 1-31
  int32 hour = 4;      // 0-23
  int32 minute = 5;
  int32 second = 6;
  bool has_time = 7;   // false for a date-only cell
}
```

## Columns

`excel_id` is the spreadsheet column letter, not a zero-based index. `name` is the trimmed text of the header cell. A header cell that is empty does not become a column. Two header cells with the same trimmed name are an error at header selection: the PDF binding addresses columns by name, so the name has to identify one column. When a column has `subcolumns`, those child names only have to be unique under that parent. The same child name under a different parent is a different column.

Cells in a row follow the leaf columns. A column with no subcolumns is itself a leaf. A column with subcolumns contributes those subcolumns and not a second cell for the parent name. A missing `excel_id` means that cell was empty. Empty cells are omitted rather than sent as a blank string.

## Values

Calc stores every number as a double and stores a date as a number plus a format. The reader classifies each non-empty cell:

| Cell | `Value` |
|---|---|
| Text | `text` |
| Boolean | `bool_value` |
| Date, time, or date-time format | `date`. Time-only still fills year, month, and day from the serial, and `has_time` is true |
| Whole number whose format is a plain number | `int_value` |
| Any other number, including percent and currency | `float_value` (the numeric value, not the formatted text with a symbol) |
| Formula | the calculated result, then the same classification. The formula string is not returned |
| Error such as `#N/A` | `text` holding the error token |

A whole number is a value within `1e-9` of an integer that fits in `int64`. Dates are civil values with no time zone. Excel and Calc serials are both timezone-naive; the message does not convert them to an instant.

## Who writes which fields

`excel-tooling` fills `excel_id`, `excel_row`, and the value kinds above. A later Google Sheets or Yandex Tables adapter fills the same messages. Those adapters still send a column letter in `excel_id` (A, B, C, by position) so the PDF service does not grow a second key. `pdf-tooling` reads `columns[].name` and the cell values. It does not interpret `excel_id` except to pair a cell with its column.
