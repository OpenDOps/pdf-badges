from excel_tooling.workbook import _OPEN_PROPERTIES


def test_open_disables_macros_and_link_updates():
    props = dict(_OPEN_PROPERTIES)
    assert props["Hidden"] is True
    assert props["MacroExecutionMode"] == 0
    assert props["UpdateDocMode"] == 0
