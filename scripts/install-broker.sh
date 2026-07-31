#!/usr/bin/env sh
set -eu

SILO_BIN=${SILO_CLI_BIN:-"$(command -v silo 2>/dev/null || true)"}
VAULT_PATH=${1:-"$HOME/silo.vault"}
TIMEOUT=${SILO_TIMEOUT:-900}

if [ -z "$SILO_BIN" ] || [ ! -x "$SILO_BIN" ]; then
  echo "Silo CLI not found. Set SILO_CLI_BIN to the installed silo binary." >&2
  exit 1
fi

case "$(uname -s)" in
  Darwin)
    LABEL=com.silo.broker
    PLIST_DIR="$HOME/Library/LaunchAgents"
    PLIST="$PLIST_DIR/$LABEL.plist"
    mkdir -p "$PLIST_DIR"
    python3 - "$PLIST" "$SILO_BIN" "$VAULT_PATH" "$TIMEOUT" <<'PY'
import pathlib
import sys

plist, binary, vault, timeout = sys.argv[1:]
pathlib.Path(plist).write_text(f'''<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.silo.broker</string>
<key>ProgramArguments</key><array>
<string>{binary}</string><string>--vault</string><string>{vault}</string>
<string>broker</string><string>--background</string><string>--timeout</string><string>{timeout}</string>
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
<key>StandardOutPath</key><string>/tmp/silo-broker.log</string>
<key>StandardErrorPath</key><string>/tmp/silo-broker.log</string>
</dict></plist>
''')
PY
    launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    echo "Installed Silo broker LaunchAgent for $VAULT_PATH"
    ;;
  Linux)
    UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    UNIT="$UNIT_DIR/silo-broker.service"
    mkdir -p "$UNIT_DIR"
    python3 - "$UNIT" "$SILO_BIN" "$VAULT_PATH" "$TIMEOUT" <<'PY'
import pathlib
import sys

unit, binary, vault, timeout = sys.argv[1:]
pathlib.Path(unit).write_text(f'''[Unit]
Description=Silo local password broker

[Service]
ExecStart={binary} --vault {vault} broker --background --timeout {timeout}
Restart=on-failure

[Install]
WantedBy=default.target
''')
PY
    systemctl --user daemon-reload
    systemctl --user enable --now silo-broker.service
    echo "Installed Silo user service for $VAULT_PATH"
    ;;
  *)
    echo "Use scripts/install-broker.ps1 on Windows." >&2
    exit 1
    ;;
esac
