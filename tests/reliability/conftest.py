"""Make the reliability cron script importable from these tests.

`scripts/reliability/reliability_report.py` is a standalone script, not an
installed package (its `pyproject.toml` sets `packages = []`), so its directory
has to go on `sys.path` before `import reliability_report` resolves.
"""

import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parents[2] / "scripts" / "reliability"

if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
