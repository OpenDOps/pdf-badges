"""Internal table. Protobuf conversion happens at the gRPC boundary."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Column:
    index: int
    excel_id: str
    name: str
    subcolumns: tuple[Column, ...] = ()


@dataclass(frozen=True)
class DateValue:
    year: int
    month: int
    day: int
    hour: int = 0
    minute: int = 0
    second: int = 0
    has_time: bool = False


@dataclass(frozen=True)
class CellValue:
    """One of text, float, int, date, or bool."""

    text: str | None = None
    float_value: float | None = None
    int_value: int | None = None
    date: DateValue | None = None
    bool_value: bool | None = None


@dataclass(frozen=True)
class GridCell:
    col: int
    display: str
    value: CellValue | None = None


@dataclass
class Grid:
    first_row: int
    last_row: int
    first_col: int
    last_col: int
    cells: dict[int, list[GridCell]] = field(default_factory=dict)


@dataclass(frozen=True)
class SheetInfo:
    index: int
    name: str


@dataclass(frozen=True)
class Candidate:
    excel_row: int
    columns: tuple[Column, ...]


@dataclass(frozen=True)
class DataRow:
    excel_row: int
    cells: tuple[GridCell, ...]
