"""Load the shared library and read the queues the Rust test arranged."""

import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from native_print import NativePrintError, queues


def main():
    service = os.environ.get("NATIVE_PRINT_SERVICE")
    dead = os.environ.get("NATIVE_PRINT_DEAD")
    found = queues(service)
    if service:
        by_name = {queue.name: queue for queue in found}
        zebra = by_name["zebra"]
        assert zebra.kind == "zpl", zebra.kind
        assert zebra.dpi == 300, zebra.dpi
        assert zebra.host == "", zebra.host
        assert zebra.installed is True
        assert zebra.connected is False
        ipp = by_name["ipp"]
        assert ipp.host == "192.0.2.10", ipp.host
        assert ipp.device_uri == "ipp://192.0.2.10/ipp/print"
        evolis = by_name["evolis"]
        assert evolis.kind == "raster", evolis.kind
        assert evolis.width_in == "2.125", evolis.width_in
    else:
        for queue in found:
            assert queue.kind in ("zpl", "raster"), queue.kind
            assert queue.name
    if dead:
        try:
            queues(dead)
        except NativePrintError as err:
            assert "connecting" in str(err), err
        else:
            raise SystemExit("a closed service returned queues")


if __name__ == "__main__":
    main()
