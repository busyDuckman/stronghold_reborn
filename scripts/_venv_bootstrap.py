"""Re-exec the current script through the project's .venv python.

Usage (must be the first thing a script does, before any third-party imports):

    #!/usr/bin/env python3
    from _venv_bootstrap import run_in_venv
    run_in_venv()

    # ... rest of imports / code ...

If we are already inside a venv, this is a no-op. Otherwise we look for a
sibling .venv directory (walking up from the script) and re-exec ourselves
through its python. Stdlib-only so it is safe to import before deps exist.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

_VENV_CANDIDATES = (".venv", "venv", ".venv_linux", ".venv_win10")


def _find_venv_python(start: Path) -> Path | None:
    for parent in (start, *start.parents):
        for name in _VENV_CANDIDATES:
            for sub in ("bin/python3", "bin/python", "Scripts/python.exe"):
                candidate = parent / name / sub
                if candidate.is_file():
                    return candidate
    return None


def run_in_venv() -> None:
    if sys.prefix != sys.base_prefix:
        return
    script = Path(sys.argv[0]).resolve()
    venv_python = _find_venv_python(script.parent)
    if venv_python is None:
        sys.stderr.write(
            "no project .venv found; run `just setup` or activate a venv first\n"
        )
        sys.exit(1)
    os.execv(str(venv_python), [str(venv_python), *sys.argv])