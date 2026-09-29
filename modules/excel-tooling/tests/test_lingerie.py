from pathlib import Path

import pytest

from excel_tooling.model import CellValue
from excel_tooling.scan import DuplicateColumn
from excel_tooling.workbook import open

pytestmark = pytest.mark.libreoffice

FIXTURES = Path(__file__).resolve().parent / "fixtures"


@pytest.fixture(scope="module")
def lingerie():
    book = open(FIXTURES / "JPGVL0I5VW.xlsm")
    yield book
    book.close()


def _at_id(row, columns, excel_id):
    column = next(item for item in columns if item.excel_id == excel_id)
    return next(cell for cell in row.cells if cell.col == column.index)


def test_lingerie_sheets(lingerie):
    assert [(sheet.index, sheet.name) for sheet in lingerie.sheets()] == [
        (0, "База посетителей"),
        (1, "prop"),
    ]
    assert lingerie.selected_sheet_index == 0


def test_lingerie_candidates(lingerie):
    assert [candidate.excel_row for candidate in lingerie.candidates(0, 3)] == [1, 2, 3]
    assert [candidate.excel_row for candidate in lingerie.candidates(0, 10)] == list(range(1, 11))


def test_lingerie_short_ids(lingerie):
    columns, rows = lingerie.read_table(0, 1, 3)
    assert len(columns) == 109
    assert (columns[0].excel_id, columns[0].name) == ("A", "h0")
    assert (columns[-1].excel_id, columns[-1].name) == ("DE", "daysvisit")
    assert rows[0].excel_row == 2
    assert _at_id(rows[0], columns, "A").value == CellValue(text="ID")


def test_lingerie_full_names(lingerie):
    columns, rows = lingerie.read_table(0, 2, 3)
    assert len(columns) == 65
    assert (columns[0].excel_id, columns[0].name) == ("A", "ID")
    assert next(column.name for column in columns if column.excel_id == "AH") == "Дата"
    assert next(column.name for column in columns if column.excel_id == "DC") == (
        "Область проживания (Не из списка)"
    )
    assert all(column.excel_id != "AS" for column in columns)
    assert len(rows) == 2924
    assert rows[0].excel_row == 3
    visitor = next(row for row in rows if row.excel_row == 4)
    for excel_id, text in (("A", "JPGVL0I5VW_exp6"), ("E", "1712460"), ("AH", "02.09.2026")):
        value = _at_id(visitor, columns, excel_id).value
        assert value == CellValue(text=text)


def test_lingerie_group_labels(lingerie):
    with pytest.raises(DuplicateColumn):
        lingerie.select_header(0, 3, 3)


def test_lingerie_subcolumns(lingerie):
    columns, rows = lingerie.read_table(0, 2, 3, subcolumn_row=3)
    assert len(columns) == 65
    parent = next(column for column in columns if column.excel_id == "AR")
    assert parent.name == "Как Вы узнали о LINGERIE SHOW-FORUM?"
    assert [(column.excel_id, column.name) for column in parent.subcolumns[:1]] == [
        ("AR", "Реклама на сайтах")
    ]
    assert (parent.subcolumns[-1].excel_id, parent.subcolumns[-1].name) == ("BG", "Свой вариант")
    assert len(parent.subcolumns) == 16
    after = columns[columns.index(parent) + 1]
    assert (after.excel_id, after.name) == ("BH", "Даты участия (эксп)")
    assert after.subcolumns == ()
    other = [
        column.name
        for column in columns
        if any(sub.name == "Другое" for sub in column.subcolumns)
    ]
    assert len(other) > 1
    assert rows[0].excel_row == 4
    assert len(rows) == 2923
    for excel_id, text in (("A", "JPGVL0I5VW_exp6"), ("E", "1712460"), ("AH", "02.09.2026")):
        assert _at_id(rows[0], columns, excel_id).value == CellValue(text=text)


def test_lingerie_prop(lingerie):
    columns = lingerie.candidates(1, 3)[0].columns
    assert ("A", "headerLines") in {(column.excel_id, column.name) for column in columns}
    assert ("E", "colFullName") in {(column.excel_id, column.name) for column in columns}
