import json
import pathlib
import sys
import tomllib
root = pathlib.Path(__file__).resolve().parents[1]
version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
gui = root / "apps/agent-usage-gui"
for name in ("package.json", "package-lock.json"):
    data = json.loads((gui / name).read_text())
    assert data["version"] == version, f"{name} version differs from {version}"
    if name == "package-lock.json":
        assert data["packages"][""]["version"] == version
if len(sys.argv) > 1:
    assert sys.argv[1] == f"v{version}", "Tag must match workspace version"
print(f"Shared version: {version}")
