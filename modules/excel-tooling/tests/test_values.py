import datetime as dt

from excel_tooling.model import CellValue, DateValue
from excel_tooling.values import Reading, classify


def serial_for(day: dt.date, null: dt.date, *, hour: int = 0, minute: int = 0, second: int = 0) -> float:
    days = (day - null).days
    return days + (hour * 3600 + minute * 60 + second) / 86400


NULL_1900 = dt.date(1899, 12, 30)
NULL_1904 = dt.date(1904, 1, 1)
DAY = dt.date(2024, 3, 15)


def test_text_stays_text():
    assert classify(Reading("text", display="0012")) == CellValue(text="0012")


def test_whole_number():
    assert classify(Reading("number", serial=2)) == CellValue(int_value=2)


def test_fraction():
    assert classify(Reading("number", serial=2.5)) == CellValue(float_value=2.5)


def test_near_integer():
    assert classify(Reading("number", serial=3 + 1e-12)) == CellValue(int_value=3)


def test_percent():
    assert classify(Reading("number", serial=0.5, is_percent=True)) == CellValue(float_value=0.5)


def test_currency():
    assert classify(Reading("number", serial=1.5, is_currency=True)) == CellValue(float_value=1.5)


def test_date():
    value = classify(
        Reading(
            "number",
            serial=serial_for(DAY, NULL_1900),
            is_date=True,
            null_year=1899,
            null_month=12,
            null_day=30,
        )
    )
    assert value == CellValue(date=DateValue(2024, 3, 15, has_time=False))


def test_date_time():
    value = classify(
        Reading(
            "number",
            serial=serial_for(DAY, NULL_1900, hour=13, minute=45),
            is_date=True,
            is_time=True,
            null_year=1899,
            null_month=12,
            null_day=30,
        )
    )
    assert value == CellValue(date=DateValue(2024, 3, 15, 13, 45, 0, has_time=True))


def test_time_keeps_the_day():
    value = classify(
        Reading(
            "number",
            serial=serial_for(DAY, NULL_1900, hour=13, minute=45),
            is_time=True,
            null_year=1899,
            null_month=12,
            null_day=30,
        )
    )
    assert value == CellValue(date=DateValue(2024, 3, 15, 13, 45, 0, has_time=True))


def test_epoch_1904():
    value = classify(
        Reading(
            "number",
            serial=serial_for(DAY, NULL_1904),
            is_date=True,
            null_year=1904,
            null_month=1,
            null_day=1,
        )
    )
    assert value == CellValue(date=DateValue(2024, 3, 15, has_time=False))


def test_bool():
    assert classify(Reading("number", serial=1, is_logical=True)) == CellValue(bool_value=True)
    assert classify(Reading("number", serial=0, is_logical=True)) == CellValue(bool_value=False)


def test_formula_number():
    assert classify(Reading("formula", display="4", serial=4)) == CellValue(int_value=4)


def test_formula_blank():
    assert classify(Reading("formula", display="", serial=0, error_code=0)) is None


def test_error_token():
    assert classify(Reading("error", display="#N/A")) == CellValue(text="#N/A")


def test_date_is_not_an_int():
    value = classify(
        Reading(
            "number",
            serial=serial_for(DAY, NULL_1900),
            is_date=True,
            null_year=1899,
            null_month=12,
            null_day=30,
        )
    )
    assert value is not None
    assert value.int_value is None
    assert value.date == DateValue(2024, 3, 15, has_time=False)
