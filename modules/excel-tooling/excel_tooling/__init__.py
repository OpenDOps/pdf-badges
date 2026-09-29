"""Excel tooling."""

import sys
from pathlib import Path

_GEN = Path(__file__).resolve().parent / "gen"
if _GEN.is_dir():
    _gen_path = str(_GEN)
    if _gen_path not in sys.path:
        sys.path.insert(0, _gen_path)
