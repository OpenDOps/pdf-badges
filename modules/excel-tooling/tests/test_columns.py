from excel_tooling.columns import excel_id, parse_excel_id


def test_letters():
    assert [excel_id(index) for index in (0, 25, 26, 27)] == ["A", "Z", "AA", "AB"]


def test_parse_letters():
    assert parse_excel_id("A") == 0
    assert parse_excel_id(" a ") == 0
    assert parse_excel_id("AA") == 26
    assert parse_excel_id("XFD") == 16383
    assert excel_id(parse_excel_id("AB")) == "AB"


def test_parse_rejects_non_letters():
    assert parse_excel_id("") is None
    assert parse_excel_id("A1") is None
    assert parse_excel_id("XFE") is None
