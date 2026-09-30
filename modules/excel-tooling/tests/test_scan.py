import pytest

from excel_tooling.model import CellValue, Grid, GridCell
from excel_tooling.scan import (
    DuplicateColumn,
    HeaderNotCandidate,
    SubcolumnRowOutOfRange,
    columns_for_header,
    data_rows,
    header_candidates,
)


def text(col: int, display: str) -> GridCell:
    return GridCell(col, display, CellValue(text=display))


def grid(last_row: int, last_col: int, rows: dict[int, list[GridCell]]) -> Grid:
    return Grid(1, last_row, 0, last_col, rows)


def names(columns):
    return [(column.excel_id, column.name) for column in columns]


def test_titles_candidates():
    sheet = grid(
        4,
        2,
        {
            2: [text(0, "Quarter report")],
            4: [text(0, "Name"), text(1, "City"), text(2, "Amount")],
        },
    )
    candidates = header_candidates(sheet, 3)
    assert [candidate.excel_row for candidate in candidates] == [2, 4]
    assert names(candidates[1].columns) == [("A", "Name"), ("B", "City"), ("C", "Amount")]


def test_candidates_ignore_the_empty_span_after_the_last_value():
    sheet = grid(1_048_576, 0, {2: [text(0, "Name")]})
    assert [candidate.excel_row for candidate in header_candidates(sheet, 10)] == [2]


def test_k_limits_candidates():
    sheet = grid(4, 0, {row: [text(0, f"r{row}")] for row in range(1, 5)})
    candidates = header_candidates(sheet, 2)
    assert [candidate.excel_row for candidate in candidates] == [1, 2]


def test_blank_header_cell():
    sheet = grid(1, 2, {1: [text(0, "Name"), text(2, "Amount")]})
    columns = columns_for_header(sheet, 1, header_candidates(sheet, 1))
    assert names(columns) == [("A", "Name"), ("C", "Amount")]


def test_whitespace_name():
    sheet = grid(1, 0, {1: [text(0, "   ")]})
    assert header_candidates(sheet, 1) == ()


def test_duplicate_names():
    sheet = grid(1, 1, {1: [text(0, "Name"), text(1, "Name")]})
    candidates = header_candidates(sheet, 1)
    with pytest.raises(DuplicateColumn):
        columns_for_header(sheet, 1, candidates)


def test_header_must_be_candidate():
    sheet = grid(2, 0, {2: [text(0, "Name")]})
    candidates = header_candidates(sheet, 3)
    with pytest.raises(HeaderNotCandidate):
        columns_for_header(sheet, 1, candidates)


def test_titles_data():
    sheet = grid(
        7,
        2,
        {
            2: [text(0, "Quarter report")],
            4: [text(0, "Name"), text(1, "City"), text(2, "Amount")],
            5: [text(0, "Ann"), text(1, "Oslo"), text(2, "2")],
            7: [text(0, "Bo"), text(2, "3")],
        },
    )
    columns = columns_for_header(sheet, 4, header_candidates(sheet, 3))
    rows = data_rows(sheet, 4, columns)
    assert [row.excel_row for row in rows] == [5, 7]
    assert [cell.display for cell in rows[0].cells] == ["Ann", "Oslo", "2"]
    assert [cell.col for cell in rows[1].cells] == [0, 2]
    assert [cell.display for cell in rows[1].cells] == ["Bo", "3"]


def test_title_rows_are_not_data():
    sheet = grid(
        5,
        2,
        {
            2: [text(0, "Quarter report")],
            4: [text(0, "Name"), text(1, "City"), text(2, "Amount")],
            5: [text(0, "Ann"), text(1, "Oslo"), text(2, "2")],
        },
    )
    columns = columns_for_header(sheet, 4, header_candidates(sheet, 3))
    assert [row.excel_row for row in data_rows(sheet, 4, columns)] == [5]


def test_gap_stops():
    rows = {1: [text(0, "Name")], 2: [text(0, "Keep")], 53: [text(0, "Drop")]}
    sheet = grid(53, 0, rows)
    columns = columns_for_header(sheet, 1, header_candidates(sheet, 1))
    assert [row.excel_row for row in data_rows(sheet, 1, columns)] == [2]


def test_gap_allows_49():
    rows = {1: [text(0, "Name")], 2: [text(0, "Keep")], 52: [text(0, "Keep")]}
    sheet = grid(52, 0, rows)
    columns = columns_for_header(sheet, 1, header_candidates(sheet, 1))
    assert [row.excel_row for row in data_rows(sheet, 1, columns)] == [2, 52]


def test_stray_column():
    sheet = grid(
        2,
        3,
        {
            1: [text(0, "Name"), text(1, "City")],
            2: [text(3, "note")],
        },
    )
    columns = columns_for_header(sheet, 1, header_candidates(sheet, 1))
    assert data_rows(sheet, 1, columns) == ()


def test_subcolumns_follow_empty_parent_cells():
    sheet = grid(
        4,
        3,
        {
            1: [text(0, "Group"), text(3, "Next")],
            2: [text(0, "One"), text(1, "Two"), text(2, "Other")],
            4: [text(0, "a"), text(1, "b"), text(3, "n")],
        },
    )
    columns = columns_for_header(sheet, 1, header_candidates(sheet, 2), subcolumn_row=2)
    group, nxt = columns
    assert (group.excel_id, group.name) == ("A", "Group")
    assert [(column.excel_id, column.name) for column in group.subcolumns] == [
        ("A", "One"),
        ("B", "Two"),
        ("C", "Other"),
    ]
    assert nxt.subcolumns == ()
    assert (nxt.excel_id, nxt.name) == ("D", "Next")
    rows = data_rows(sheet, 1, columns, subcolumn_row=2)
    assert [row.excel_row for row in rows] == [4]
    assert [cell.display for cell in rows[0].cells] == ["a", "b", "n"]


def test_same_subcolumn_name_under_two_parents():
    sheet = grid(
        2,
        3,
        {
            1: [text(0, "Left"), text(2, "Right")],
            2: [text(0, "Other"), text(1, "Own"), text(2, "Other"), text(3, "Own")],
        },
    )
    columns = columns_for_header(sheet, 1, header_candidates(sheet, 1), subcolumn_row=2)
    assert [column.name for column in columns[0].subcolumns] == ["Other", "Own"]
    assert [column.name for column in columns[1].subcolumns] == ["Other", "Own"]


def test_duplicate_subcolumn_under_one_parent():
    sheet = grid(
        2,
        2,
        {
            1: [text(0, "Group"), text(2, "Next")],
            2: [text(0, "Other"), text(1, "Other")],
        },
    )
    with pytest.raises(DuplicateColumn):
        columns_for_header(sheet, 1, header_candidates(sheet, 1), subcolumn_row=2)


def test_subcolumn_row_window():
    sheet = grid(6, 0, {1: [text(0, "Name")], 5: [text(0, "Under")], 6: [text(0, "Value")]})
    candidates = header_candidates(sheet, 3)
    columns = columns_for_header(sheet, 1, candidates, subcolumn_row=5)
    assert columns[0].subcolumns[0].name == "Under"
    assert [row.excel_row for row in data_rows(sheet, 1, columns, subcolumn_row=5)] == [6]
    with pytest.raises(SubcolumnRowOutOfRange):
        columns_for_header(sheet, 1, candidates, subcolumn_row=6)
