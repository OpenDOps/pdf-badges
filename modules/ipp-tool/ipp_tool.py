"""Thin embedding of the ipp-tool shared library."""

import ctypes
import sys
from pathlib import Path


class IppToolError(RuntimeError):
    pass


def library_path():
    env = __import__("os").environ.get("IPP_TOOL_LIB")
    if env:
        return Path(env)
    filename = {
        "darwin": "libipp_tool.dylib",
        "win32": "ipp_tool.dll",
    }.get(sys.platform, "libipp_tool.so")
    root = Path(__file__).resolve().parents[2]
    for folder in (root / "target" / "debug", root / "target" / "release"):
        candidate = folder / filename
        if candidate.is_file():
            return candidate
    raise IppToolError(
        "shared library not found; set IPP_TOOL_LIB or cargo build -p ipp-tool"
    )


def _load():
    lib = ctypes.CDLL(str(library_path()))
    lib.ipp_tool_printer_open.argtypes = [
        ctypes.c_char_p,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    lib.ipp_tool_printer_open.restype = ctypes.c_int
    lib.ipp_tool_info.argtypes = [ctypes.c_void_p]
    lib.ipp_tool_info.restype = ctypes.c_int
    lib.ipp_tool_make_model.argtypes = [ctypes.c_void_p]
    lib.ipp_tool_make_model.restype = ctypes.c_char_p
    lib.ipp_tool_set.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.ipp_tool_set.restype = ctypes.c_int
    lib.ipp_tool_print_test.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int)]
    lib.ipp_tool_print_test.restype = ctypes.c_int
    lib.ipp_tool_print_file.argtypes = [
        ctypes.c_void_p,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.POINTER(ctypes.c_int),
    ]
    lib.ipp_tool_print_file.restype = ctypes.c_int
    lib.ipp_tool_jobs.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_size_t),
    ]
    lib.ipp_tool_jobs.restype = ctypes.c_int
    lib.ipp_tool_cancel.argtypes = [ctypes.c_void_p, ctypes.c_int]
    lib.ipp_tool_cancel.restype = ctypes.c_int
    lib.ipp_tool_hold.argtypes = [ctypes.c_void_p, ctypes.c_int]
    lib.ipp_tool_hold.restype = ctypes.c_int
    lib.ipp_tool_release.argtypes = [ctypes.c_void_p, ctypes.c_int]
    lib.ipp_tool_release.restype = ctypes.c_int
    lib.ipp_tool_discover.argtypes = [
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_size_t),
    ]
    lib.ipp_tool_discover.restype = ctypes.c_int
    lib.ipp_tool_refresh.argtypes = [
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_size_t),
    ]
    lib.ipp_tool_refresh.restype = ctypes.c_int
    lib.ipp_tool_add.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
    lib.ipp_tool_add.restype = ctypes.c_int
    lib.ipp_tool_printer_free.argtypes = [ctypes.c_void_p]
    lib.ipp_tool_printer_free.restype = None
    lib.ipp_tool_list_free.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    lib.ipp_tool_list_free.restype = None
    lib.ipp_tool_last_error.argtypes = []
    lib.ipp_tool_last_error.restype = ctypes.c_char_p
    return lib


class Printer:
    def __init__(self, uri):
        self._lib = _load()
        handle = ctypes.c_void_p()
        code = self._lib.ipp_tool_printer_open(str(uri).encode(), ctypes.byref(handle))
        if code != 0:
            raise IppToolError(self._error())
        self._handle = handle

    def info(self):
        code = self._lib.ipp_tool_info(self._handle)
        if code != 0:
            raise IppToolError(self._error())

    @property
    def make_model(self):
        raw = self._lib.ipp_tool_make_model(self._handle)
        if raw is None:
            return ""
        return raw.decode()

    def set(self, setting, value):
        code = self._lib.ipp_tool_set(
            self._handle, str(setting).encode(), str(value).encode()
        )
        if code != 0:
            raise IppToolError(self._error())

    def print_test(self):
        return self._job(self._lib.ipp_tool_print_test)

    def print_file(self, path, raw=0):
        job_id = ctypes.c_int()
        code = self._lib.ipp_tool_print_file(
            self._handle, str(path).encode(), int(raw), ctypes.byref(job_id)
        )
        if code != 0:
            raise IppToolError(self._error())
        return job_id.value

    def jobs(self):
        out = ctypes.c_void_p()
        count = ctypes.c_size_t()
        code = self._lib.ipp_tool_jobs(
            self._handle, ctypes.byref(out), ctypes.byref(count)
        )
        if code != 0:
            raise IppToolError(self._error())
        return []

    def cancel(self, job_id):
        self._job_id(self._lib.ipp_tool_cancel, job_id)

    def hold(self, job_id):
        self._job_id(self._lib.ipp_tool_hold, job_id)

    def release(self, job_id):
        self._job_id(self._lib.ipp_tool_release, job_id)

    def close(self):
        handle = getattr(self, "_handle", None)
        if handle is not None and handle.value:
            self._lib.ipp_tool_printer_free(handle)
            self._handle = None

    def _job(self, call):
        job_id = ctypes.c_int()
        code = call(self._handle, ctypes.byref(job_id))
        if code != 0:
            raise IppToolError(self._error())
        return job_id.value

    def _job_id(self, call, job_id):
        code = call(self._handle, int(job_id))
        if code != 0:
            raise IppToolError(self._error())

    def _error(self):
        raw = self._lib.ipp_tool_last_error()
        if raw is None:
            return "ipp tool failed"
        return raw.decode()

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        self.close()
