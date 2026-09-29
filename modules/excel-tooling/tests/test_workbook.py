from pathlib import Path

import pytest

from excel_tooling.model import CellValue, DateValue
from excel_tooling.scan import DuplicateColumn
from excel_tooling.workbook import BadFile, NoData, SheetNotListed, open

pytestmark = pytest.mark.libreoffice

FIXTURES = Path(__file__).resolve().parent / "fixtures"


@pytest.fixture(scope="module")
def titles():
    book = open(FIXTURES / "titles.xlsx")
    yield book
    book.close()


def _column(columns, name):
    return next(column for column in columns if column.name == name)


def _at(row, column):
    return next(cell for cell in row.cells if cell.col == column.index)


def test_lists_non_empty_sheets(titles):
    assert [(sheet.index, sheet.name) for sheet in titles.sheets()] == [
        (1, "Titles"),
        (2, "Types"),
        (3, "Gap"),
        (4, "Dup"),
        (5, "Wide"),
        (6, "Hidden"),
    ]
    assert titles.selected_sheet_index == 1


def test_titles_table(titles):
    columns, rows = titles.read_table(1, 4, 10)
    assert [column.name for column in columns] == ["Name", "City", "Amount", "When", "Active"]
    assert [row.excel_row for row in rows] == [5, 7]
    assert _at(rows[0], _column(columns, "Amount")).value == CellValue(int_value=2)
    assert _at(rows[0], _column(columns, "When")).value == CellValue(
        date=DateValue(2024, 3, 15, has_time=False)
    )
    assert _at(rows[0], _column(columns, "Active")).value == CellValue(bool_value=True)
    city = _column(columns, "City")
    assert all(cell.col != city.index for cell in rows[1].cells)
    assert _at(rows[1], _column(columns, "Amount")).value == CellValue(float_value=3.5)
    assert _at(rows[1], _column(columns, "When")).value == CellValue(int_value=2)


def test_types_row(titles):
    columns, rows = titles.read_table(2, 1, 1)
    values = {column.name: _at(rows[0], column).value for column in columns}
    assert values["Text"] == CellValue(text="0012")
    assert values["Whole"] == CellValue(int_value=2)
    assert values["Fraction"] == CellValue(float_value=2.5)
    assert values["Percent"] == CellValue(float_value=0.5)
    assert values["Money"] == CellValue(float_value=1.5)
    assert values["Date"] == CellValue(date=DateValue(2024, 3, 15, has_time=False))
    assert values["DateTime"] == CellValue(date=DateValue(2024, 3, 15, 13, 45, 0, has_time=True))
    assert values["Time"].date.has_time is True
    assert (values["Time"].date.hour, values["Time"].date.minute, values["Time"].date.second) == (
        13,
        45,
        0,
    )
    assert values["Flag"] == CellValue(bool_value=True)
    assert values["Formula"] == CellValue(int_value=4)
    assert values["Err"] == CellValue(text="#N/A")


def test_gap(titles):
    columns, rows = titles.read_table(3, 1, 1)
    assert [cell.display for row in rows for cell in row.cells] == ["Keep"]
    assert _column(columns, "Name").excel_id == "A"


def test_duplicate_header(titles):
    with pytest.raises(DuplicateColumn):
        titles.select_header(4, 1, 1)


def test_wide_column(titles):
    columns = titles.select_header(5, 1, 1)
    assert columns[0].excel_id == "AA"
    assert columns[0].name == "Wide"


def test_hidden_sheet(titles):
    columns = titles.select_header(6, 1, 1)
    assert columns[0].name == "Secret"


def test_sheet_not_listed(titles):
    with pytest.raises(SheetNotListed):
        titles.candidates(0, 1)


def test_epoch():
    book = open(FIXTURES / "epoch1904.xlsx")
    try:
        columns, rows = book.read_table(0, 1, 1)
        assert _at(rows[0], columns[0]).value == CellValue(date=DateValue(2024, 3, 15, has_time=False))
    finally:
        book.close()


def test_rejects_csv(tmp_path):
    path = tmp_path / "notes.csv"
    path.write_text("a,b\n", encoding="utf-8")
    with pytest.raises(BadFile):
        open(path)


def test_empty_workbook():
    with pytest.raises(NoData):
        open(FIXTURES / "empty.xlsx")
