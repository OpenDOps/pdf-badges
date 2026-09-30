"""Detect an encrypted workbook without asking Calc for a password."""

import zipfile
from pathlib import Path

_OLE = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"
_ZIP = b"PK"


def needs_password(path: str | Path) -> bool:
    source = Path(path)
    suffix = source.suffix.lower()
    with source.open("rb") as stored:
        head = stored.read(8)
    if suffix in {".xlsx", ".xlsm"}:
        return head.startswith(_OLE)
    if suffix == ".ods":
        return head.startswith(_ZIP) and _ods_encrypted(source)
    return False


def _ods_encrypted(path: Path) -> bool:
    try:
        with zipfile.ZipFile(path) as package:
            manifest = package.read("META-INF/manifest.xml")
    except (zipfile.BadZipFile, KeyError, OSError):
        return False
    return b"encryption-data" in manifest
