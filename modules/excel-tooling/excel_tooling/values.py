"""Cell classification."""

import datetime as dt
import math

from excel_tooling.model import CellValue, DateValue

_INT64_MIN = -9223372036854775808
_INT64_MAX = 9223372036854775807


class Reading:
    """One cell as Calc reports it, before classification."""

    def __init__(
        self,
        kind: str,
        display: str = "",
        serial: float = 0.0,
        error_code: int = 0,
        *,
        is_date: bool = False,
        is_time: bool = False,
        is_logical: bool = False,
        is_percent: bool = False,
        is_currency: bool = False,
        null_year: int = 1899,
        null_month: int = 12,
        null_day: int = 30,
    ):
        self.kind = kind
        self.display = display
        self.serial = serial
        self.error_code = error_code
        self.is_date = is_date
        self.is_time = is_time
        self.is_logical = is_logical
        self.is_percent = is_percent
        self.is_currency = is_currency
        self.null_year = null_year
        self.null_month = null_month
        self.null_day = null_day


def classify(reading: Reading) -> CellValue | None:
    if reading.kind == "empty":
        return None
    if reading.kind == "formula" and reading.display == "" and reading.error_code == 0:
        return None
    if reading.kind == "error" or (reading.kind == "formula" and reading.error_code != 0):
        return CellValue(text=reading.display)
    if reading.kind == "text":
        return CellValue(text=reading.display)
    if reading.is_date or reading.is_time:
        return CellValue(date=_date_value(reading))
    if reading.is_logical:
        return CellValue(bool_value=reading.serial != 0)
    if reading.is_percent or reading.is_currency:
        return CellValue(float_value=reading.serial)
    nearest = round(reading.serial)
    if abs(reading.serial - nearest) < 1e-9 and _INT64_MIN <= nearest <= _INT64_MAX:
        return CellValue(int_value=nearest)
    return CellValue(float_value=reading.serial)


def _date_value(reading: Reading) -> DateValue:
    days = math.trunc(reading.serial)
    has_time = reading.is_time
    if has_time:
        seconds = round((reading.serial - days) * 86400)
        if seconds >= 86400:
            days += 1
            seconds = 0
        elif seconds < 0:
            days -= 1
            seconds += 86400
    else:
        seconds = 0
    day = dt.date(reading.null_year, reading.null_month, reading.null_day)
    day = day + dt.timedelta(days=days)
    hour, remainder = divmod(seconds, 3600)
    minute, second = divmod(remainder, 60)
    return DateValue(day.year, day.month, day.day, hour, minute, second, has_time)
