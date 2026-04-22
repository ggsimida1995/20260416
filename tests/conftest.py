import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


def make_project_dir(base: Path, name: str) -> Path:
    project_dir = base / name
    project_dir.mkdir(parents=True, exist_ok=True)
    return project_dir
