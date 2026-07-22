#!/usr/bin/env sh
set -eu

test -x scripts/install-native-host.sh
sh -n scripts/install-native-host.sh
python3 - <<'PY'
import json
from pathlib import Path

manifest = json.loads(Path("extension/native-host-manifest.example.json").read_text())
assert manifest["name"] == "com.silo.native"
assert manifest["type"] == "stdio"
assert "path" in manifest
PY

if command -v pwsh >/dev/null 2>&1; then
  pwsh -NoProfile -Command '$null = [System.Management.Automation.Language.Parser]::ParseFile("scripts/install-native-host.ps1", [ref]$null, [ref]$null)'
fi

echo "Packaging manifests and installer syntax are valid."
