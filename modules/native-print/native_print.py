"""Thin embedding of the native-print shared library.

The calls are the header in include/native_print.h. Linux and macOS load the
CUPS build. Windows loads the spooler build. The Python names do not change.
"""

import ctypes
import sys
from pathlib import Path


class NativePrintError(RuntimeError):
    pass


class Choice:
    def __init__(self, name, values):
        self.name = name
        self.values = values


class Queue:
    def __init__(
        self,
        name,
        device_uri,
        make_model,
        state,
        host,
        kind,
        dpi,
        width_in,
        accepting,
        installed,
        connected,
        choices,
    ):
        self.name = name
        self.device_uri = device_uri
        self.make_model = make_model
        self.state = state
        self.host = host
        self.kind = kind
        self.dpi = dpi
        self.width_in = width_in
        self.accepting = accepting
        self.installed = installed
        self.connected = connected
        self.choices = choices


def library_path():
    env = __import__("os").environ.get("NATIVE_PRINT_LIB")
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
    raise NativePrintError(
        "shared library not found; set NATIVE_PRINT_LIB or cargo build --lib"
    )


def _load():
    lib = ctypes.CDLL(str(library_path()))
    lib.native_print_list.argtypes = [
        ctypes.c_char_p,
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_size_t),
    ]
    lib.native_print_list.restype = ctypes.c_int
    lib.native_print_list_free.argtypes = [ctypes.c_void_p]
    lib.native_print_list_free.restype = None
    lib.native_print_last_error.argtypes = []
    lib.native_print_last_error.restype = ctypes.c_char_p
    for name in (
        "native_print_name",
        "native_print_device_uri",
        "native_print_make_model",
        "native_print_state",
        "native_print_host",
        "native_print_kind",
        "native_print_width_in",
        "native_print_choice_name",
    ):
        fn = getattr(lib, name)
        if name == "native_print_choice_name":
            fn.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_size_t]
        else:
            fn.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        fn.restype = ctypes.c_char_p
    for name in (
        "native_print_accepting",
        "native_print_dpi",
        "native_print_installed",
        "native_print_connected",
    ):
        fn = getattr(lib, name)
        fn.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        fn.restype = ctypes.c_int
    lib.native_print_choice_count.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    lib.native_print_choice_count.restype = ctypes.c_size_t
    lib.native_print_choice_value_count.argtypes = [
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_size_t,
    ]
    lib.native_print_choice_value_count.restype = ctypes.c_size_t
    lib.native_print_choice_value.argtypes = [
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_size_t,
        ctypes.c_size_t,
    ]
    lib.native_print_choice_value.restype = ctypes.c_char_p
    return lib


def _text(raw):
    if raw is None:
        return ""
    return raw.decode()


def queues(service=None):
    lib = _load()
    handle = ctypes.c_void_p()
    count = ctypes.c_size_t()
    encoded = None if service is None else str(service).encode()
    code = lib.native_print_list(encoded, ctypes.byref(handle), ctypes.byref(count))
    if code != 0:
        raise NativePrintError(_error(lib))
    found = []
    try:
        for index in range(count.value):
            choices = []
            choice_count = lib.native_print_choice_count(handle, index)
            for choice in range(choice_count):
                values = []
                value_count = lib.native_print_choice_value_count(handle, index, choice)
                for value in range(value_count):
                    values.append(
                        _text(lib.native_print_choice_value(handle, index, choice, value))
                    )
                choices.append(
                    Choice(_text(lib.native_print_choice_name(handle, index, choice)), values)
                )
            found.append(
                Queue(
                    _text(lib.native_print_name(handle, index)),
                    _text(lib.native_print_device_uri(handle, index)),
                    _text(lib.native_print_make_model(handle, index)),
                    _text(lib.native_print_state(handle, index)),
                    _text(lib.native_print_host(handle, index)),
                    _text(lib.native_print_kind(handle, index)),
                    int(lib.native_print_dpi(handle, index)),
                    _text(lib.native_print_width_in(handle, index)),
                    bool(lib.native_print_accepting(handle, index)),
                    bool(lib.native_print_installed(handle, index)),
                    bool(lib.native_print_connected(handle, index)),
                    choices,
                )
            )
    finally:
        if handle.value:
            lib.native_print_list_free(handle)
    return found


def _error(lib):
    raw = lib.native_print_last_error()
    if raw is None:
        return "native print failed"
    return raw.decode()
