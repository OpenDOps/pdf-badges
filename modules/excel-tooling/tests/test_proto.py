import excel_tooling  # noqa: F401  (puts generated stubs on sys.path)
from irbis.excel.v1 import excel_pb2, excel_pb2_grpc
from irbis.table.v1 import table_pb2


def test_proto_table_round_trip():
    table = table_pb2.Table()
    name = table.columns.add()
    name.excel_id = "A"
    name.name = "Name"
    city = table.columns.add()
    city.excel_id = "B"
    city.name = "City"
    row = table.rows.add()
    row.excel_row = 4
    cell = row.cells.add()
    cell.excel_id = "A"
    cell.value.text = "Ann"

    parsed = table_pb2.Table()
    parsed.ParseFromString(table.SerializeToString())

    assert [(column.excel_id, column.name) for column in parsed.columns] == [
        ("A", "Name"),
        ("B", "City"),
    ]
    assert parsed.rows[0].excel_row == 4
    assert parsed.rows[0].cells[0].excel_id == "A"
    assert parsed.rows[0].cells[0].value.WhichOneof("kind") == "text"
    assert parsed.rows[0].cells[0].value.text == "Ann"
    assert excel_pb2.OpenMeta(filename="titles.xlsx").filename == "titles.xlsx"
    assert hasattr(excel_pb2_grpc, "ExcelToolingStub")
