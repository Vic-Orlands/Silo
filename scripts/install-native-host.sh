#!/usr/bin/env sh
set -eu

SILO_BIN=${SILO_NATIVE_HOST_BIN:-"$(pwd)/target/debug/silo-native-host"}
EXTENSION_ID=${1:-YOUR_EXTENSION_ID}
BROWSER=${2:-chrome}
HOST_NAME=com.silo.native

case "$(uname -s):$BROWSER" in
  Darwin:firefox) HOST_DIR="$HOME/Library/Application Support/Mozilla/NativeMessagingHosts" ;;
  Darwin:chromium) HOST_DIR="$HOME/Library/Application Support/Chromium/NativeMessagingHosts" ;;
  Darwin:*) HOST_DIR="$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts" ;;
  Linux:firefox) HOST_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/mozilla/native-messaging-hosts" ;;
  Linux:chromium) HOST_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/chromium/NativeMessagingHosts" ;;
  Linux:*) HOST_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/google-chrome/NativeMessagingHosts" ;;
  *) echo "Use scripts/install-native-host.ps1 on Windows." >&2; exit 1 ;;
esac

mkdir -p "$HOST_DIR"
LAUNCHER="$HOST_DIR/silo-native-host-launcher"
python3 - "$HOST_DIR/$HOST_NAME.json" "$SILO_BIN" "$EXTENSION_ID" "$LAUNCHER" "$BROWSER" <<'PY'
import json, pathlib, sys
output, binary, extension_id, launcher, browser = sys.argv[1:]
pathlib.Path(launcher).write_text(f'#!/usr/bin/env sh\nexec {json.dumps(str(pathlib.Path(binary).expanduser().resolve()))}\n')
pathlib.Path(launcher).chmod(0o700)
manifest = {
    "name": "com.silo.native",
    "description": "Silo native messaging host",
    "path": str(pathlib.Path(launcher).expanduser().resolve()),
    "type": "stdio",
}
if browser == "firefox":
    manifest["allowed_extensions"] = [extension_id]
else:
    manifest["allowed_origins"] = [f"chrome-extension://{extension_id}/"]
pathlib.Path(output).write_text(json.dumps(manifest, indent=2) + "\n")
PY

echo "Installed $HOST_NAME for Chromium at $HOST_DIR"
echo "Browser: $BROWSER"
echo "Start the Silo broker separately with the vault you want to use."
