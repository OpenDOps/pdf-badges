"""Thin embedding of the pdf-tooling shared library."""

import ctypes
import sys
from pathlib import Path


class PdfTemplateError(RuntimeError):
    pass


class _Value(ctypes.Structure):
    _fields_ = [("field", ctypes.c_char_p), ("value", ctypes.c_char_p)]


def library_path():
    env = __import__("os").environ.get("PDF_TEMPLATE_LIB")
    if env:
        return Path(env)
    filename = {
        "darwin": "librust_reg.dylib",
        "win32": "rust_reg.dll",
    }.get(sys.platform, "librust_reg.so")
    root = Path(__file__).resolve().parents[2]
    for folder in (root / "target" / "debug", root / "target" / "release"):
        candidate = folder / filename
        if candidate.is_file():
            return candidate
    raise PdfTemplateError(
        "shared library not found; set PDF_TEMPLATE_LIB or cargo build the crate"
    )


def _load():
    path = library_path()
    lib = ctypes.CDLL(str(path))
    lib.pdf_template_index_bytes.argtypes = [
        ctypes.POINTER(ctypes.c_uint8),
        ctypes.c_size_t,
        ctypes.c_char_p,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    lib.pdf_template_index_bytes.restype = ctypes.c_int
    lib.pdf_template_field_count.argtypes = [ctypes.c_void_p]
    lib.pdf_template_field_count.restype = ctypes.c_size_t
    lib.pdf_template_field_name.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    lib.pdf_template_field_name.restype = ctypes.c_char_p
    lib.pdf_template_render.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(_Value),
        ctypes.POINTER(ctypes.c_size_t),
        ctypes.c_size_t,
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_size_t),
    ]
    lib.pdf_template_render.restype = ctypes.c_int
    lib.pdf_template_bytes_free.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    lib.pdf_template_bytes_free.restype = None
    lib.pdf_template_free.argtypes = [ctypes.c_void_p]
    lib.pdf_template_free.restype = None
    lib.pdf_template_last_error.argtypes = []
    lib.pdf_template_last_error.restype = ctypes.c_char_p
    return lib


class Template:
    def __init__(self, yaml, base_dir):
        if isinstance(yaml, str):
            yaml = yaml.encode()
        self._lib = _load()
        buf = (ctypes.c_uint8 * len(yaml)).from_buffer_copy(yaml)
        handle = ctypes.c_void_p()
        code = self._lib.pdf_template_index_bytes(
            buf, len(yaml), str(base_dir).encode(), ctypes.byref(handle)
        )
        if code != 0:
            raise PdfTemplateError(self._error())
        self._handle = handle

    def fields(self):
        count = self._lib.pdf_template_field_count(self._handle)
        names = []
        for index in range(count):
            raw = self._lib.pdf_template_field_name(self._handle, index)
            names.append(raw.decode())
        return names

    def render(self, rows):
        encoded = []
        lengths = []
        for row in rows:
            items = list(row.items())
            lengths.append(len(items))
            for field, value in items:
                encoded.append((str(field).encode(), str(value).encode()))
        values = (_Value * max(len(encoded), 1))()
        for index, (field, value) in enumerate(encoded):
            values[index].field = field
            values[index].value = value
        if rows:
            row_lengths = (ctypes.c_size_t * len(lengths))(*lengths)
        else:
            row_lengths = (ctypes.c_size_t * 1)()
        out_bytes = ctypes.c_void_p()
        out_len = ctypes.c_size_t()
        code = self._lib.pdf_template_render(
            self._handle,
            values,
            row_lengths,
            len(rows),
            ctypes.byref(out_bytes),
            ctypes.byref(out_len),
        )
        if code != 0:
            raise PdfTemplateError(self._error())
        data = ctypes.string_at(out_bytes, out_len.value)
        self._lib.pdf_template_bytes_free(out_bytes, out_len)
        return data

    def write(self, rows, path):
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(self.render(rows))

    def close(self):
        handle = getattr(self, "_handle", None)
        if handle is not None and handle.value:
            self._lib.pdf_template_free(handle)
            self._handle = None

    def _error(self):
        raw = self._lib.pdf_template_last_error()
        if raw is None:
            return "pdf template failed"
        return raw.decode()

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        self.close()
