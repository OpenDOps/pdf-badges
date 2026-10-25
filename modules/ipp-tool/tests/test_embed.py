"""Load the shared library and read a printer the Rust test is serving."""

import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from ipp_tool import IppToolError, Printer


def main():
    uri = os.environ["IPP_TOOL_URI"]
    dead = os.environ["IPP_TOOL_DEAD_URI"]
    with Printer(uri) as printer:
        printer.info()
        assert printer.make_model == "Zebra", printer.make_model
    try:
        with Printer(dead) as printer:
            printer.info()
    except IppToolError as err:
        assert "connecting" in str(err), err
    else:
        raise SystemExit("a dead uri returned a make and model")


if __name__ == "__main__":
    main()
