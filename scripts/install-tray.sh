#!/usr/bin/env sh
set -eu

SILO_TRAY_BIN=${SILO_TRAY_BIN:-"$(command -v silo-tray 2>/dev/null || true)"}
SILO_CLI_BIN=${SILO_CLI_BIN:-"$(command -v silo 2>/dev/null || true)"}
VAULT_PATH=${1:-"$HOME/silo.vault"}
TIMEOUT=${SILO_TIMEOUT:-900}

if [ -z "$SILO_TRAY_BIN" ] || [ ! -x "$SILO_TRAY_BIN" ]; then
  echo "silo-tray not found. Set SILO_TRAY_BIN to the tray binary." >&2
  exit 1
fi
if [ -z "$SILO_CLI_BIN" ] || [ ! -x "$SILO_CLI_BIN" ]; then
  echo "Silo CLI not found. Set SILO_CLI_BIN to the installed silo binary." >&2
  exit 1
fi

case "$(uname -s)" in
  Darwin)
    LABEL=com.silo.tray
    PLIST_DIR="$HOME/Library/LaunchAgents"
    PLIST="$PLIST_DIR/$LABEL.plist"
    mkdir -p "$PLIST_DIR"
    python3 - "$PLIST" "$SILO_TRAY_BIN" "$SILO_CLI_BIN" "$VAULT_PATH" "$TIMEOUT" <<'PY'
import pathlib
import sys

plist, tray, cli, vault, timeout = sys.argv[1:]
pathlib.Path(plist).write_text(f'''<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.silo.tray</string>
<key>ProgramArguments</key><array>
<string>{tray}</string><string>--vault</string><string>{vault}</string>
<string>--cli</string><string>{cli}</string><string>--timeout</string><string>{timeout}</string>
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
<key>StandardOutPath</key><string>/tmp/silo-tray.log</string>
<key>StandardErrorPath</key><string>/tmp/silo-tray.log</string>
</dict></plist>
''')
PY
    launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    echo "Installed Silo menu-bar companion for $VAULT_PATH"
    ;;
  Linux)
    UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    UNIT="$UNIT_DIR/silo-tray.service"
    mkdir -p "$UNIT_DIR"
    python3 - "$UNIT" "$SILO_TRAY_BIN" "$SILO_CLI_BIN" "$VAULT_PATH" "$TIMEOUT" <<'PY'
import pathlib
import sys

unit, tray, cli, vault, timeout = sys.argv[1:]
pathlib.Path(unit).write_text(f'''[Unit]
Description=Silo menu-bar and system-tray companion

[Service]
ExecStart={tray} --vault {vault} --cli {cli} --timeout {timeout}
Restart=on-failure

[Install]
WantedBy=default.target
''')
PY
    systemctl --user daemon-reload
    systemctl --user enable --now silo-tray.service
    echo "Installed Silo tray companion for $VAULT_PATH"
    ;;
  *)
    echo "Use scripts/install-tray.ps1 on Windows." >&2
    exit 1
    ;;
esac
