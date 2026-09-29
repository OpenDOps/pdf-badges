from excel_tooling.columns import excel_id


def test_letters():
    assert [excel_id(index) for index in (0, 25, 26, 27)] == ["A", "Z", "AA", "AB"]
