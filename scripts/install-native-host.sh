#!/usr/bin/env sh
set -eu

SILO_BIN=${SILO_NATIVE_HOST_BIN:-"$(pwd)/target/debug/silo-native-host"}
EXTENSION_ID=${1:-YOUR_EXTENSION_ID}
VAULT_PATH=${SILO_VAULT_PATH:-"$HOME/.local/share/silo/silo.vault"}
HOST_NAME=com.silo.native

case "$(uname -s)" in
  Darwin) HOST_DIR="$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts" ;;
  Linux) HOST_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/google-chrome/NativeMessagingHosts" ;;
  *) echo "Use scripts/install-native-host.ps1 on Windows." >&2; exit 1 ;;
esac

mkdir -p "$HOST_DIR"
LAUNCHER="$HOST_DIR/silo-native-host-launcher"
python3 - "$HOST_DIR/$HOST_NAME.json" "$SILO_BIN" "$EXTENSION_ID" "$VAULT_PATH" "$LAUNCHER" <<'PY'
import json, pathlib, sys
output, binary, extension_id, vault, launcher = sys.argv[1:]
pathlib.Path(launcher).write_text(f'#!/usr/bin/env sh\nexec {json.dumps(str(pathlib.Path(binary).expanduser().resolve()))} --vault {json.dumps(str(pathlib.Path(vault).expanduser().resolve()))}\n')
pathlib.Path(launcher).chmod(0o700)
pathlib.Path(output).write_text(json.dumps({
    "name": "com.silo.native",
    "description": "Silo native messaging host",
    "path": str(pathlib.Path(launcher).expanduser().resolve()),
    "type": "stdio",
    "allowed_origins": [f"chrome-extension://{extension_id}/"]
}, indent=2) + "\n")
PY

echo "Installed $HOST_NAME for Chromium at $HOST_DIR"
echo "Vault path: $VAULT_PATH"
