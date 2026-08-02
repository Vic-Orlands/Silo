---
layout: post
title: "Building Silo Tray: the background companion"
date: 2026-08-03
permalink: /building-silo-tray/
---

The terminal is Silo’s management surface, but a password manager also needs to be available when no shell is open. Silo Tray is the small menu-bar or system-tray companion that keeps the local broker available without keeping a terminal window open.

## The job of the tray

The tray is intentionally narrow. It owns the desktop lifecycle, while the broker owns the unlocked vault session.

```text
Silo Tray
├── starts a locked broker
├── reads broker status
├── exposes lock / unlock
├── opens the Silo shell
└── quits the broker session
```

It does not store a second vault, sync credentials, or become another password database.

## Process map

```text
                  ┌─────────────────┐
                  │  Silo Tray       │
                  │  menu-bar UI     │
                  └────────┬────────┘
                           │ local request
                           ▼
                  ┌─────────────────┐
                  │  Silo Broker     │
                  │  locked session  │
                  └────────┬────────┘
                           │ unlocks
                           ▼
                  ┌─────────────────┐
                  │  Encrypted vault │
                  │  local file      │
                  └─────────────────┘
```

The browser follows a separate path:

```text
Browser extension → native host → broker → approved username/password/TOTP
```

The browser never receives the master password.

## Starting the locked broker

The tray starts a broker with no decrypted vault in memory:

```rust
let broker = silo_broker::start_locked(args.vault.clone(), args.timeout)?;
```

That separation is important. A tray restart should not create a new vault format or duplicate the vault’s encryption logic. It only starts and controls the process responsible for the session.

## Menu actions

The menu is status-driven:

```text
Silo vault: locked
Unlock Silo vault
Open Silo vault
Quit Silo vault
```

When the broker is unlocked, the menu changes to:

```text
Silo vault: unlocked
Lock Silo vault
Open Silo vault
Quit Silo vault
```

The tray asks the broker for status rather than guessing from its own state:

```rust
let response = broker_request(Request::Status)?;
let label = if response.unlocked == Some(true) {
    "unlocked"
} else {
    "locked"
};
```

## Why this is a separate crate

Tray APIs are platform-specific. macOS uses a menu bar, Windows uses a notification area, and Linux depends on the desktop tray implementation. Keeping this code in `silo-tray` prevents platform UI dependencies from leaking into vault encryption or the terminal UI.

The broker stays testable without a desktop session. The tray can change its menu implementation without changing the vault file format.

## Installing it

For a local test build:

```bash
cargo build -p silo-tray

SILO_TRAY_BIN="$PWD/target/debug/silo-tray" \
SILO_CLI_BIN="$PWD/target/debug/silo" \
  sh scripts/install-tray.sh /tmp/silo-test/test.vault
```

Then verify:

1. The tray icon appears.
2. The initial status is locked.
3. Unlock changes the menu state.
4. Browser filling works without a shell window.
5. Lock removes browser access.
6. Quit stops the tray-owned broker.

## Limits

The tray does not yet provide hardware-backed unlock, sleep/wake handling on every platform, or a signed installer. Those require platform-specific testing and release work. The tray is a lifecycle companion, not a security boundary by itself.
