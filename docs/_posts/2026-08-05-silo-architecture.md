---
layout: post
title: "Silo architecture explained: why the code is split into crates"
date: 2026-08-05
permalink: /silo-architecture/
---

Silo is a Rust workspace made of several crates. This is not because every module needs to be a separate product. It is because each boundary has a different responsibility, dependency set, and testing strategy.

## Workspace map

```text
Silo workspace
├── silo-core
│   ├── vault model
│   ├── Argon2id + XChaCha20-Poly1305
│   ├── TOTP
│   └── import/export
├── silo-cli
│   ├── command-line interface
│   └── Ratatui terminal workspace
├── silo-broker
│   └── unlocked local session and request policy
├── silo-protocol
│   └── shared broker/native-host messages
├── silo-native-host
│   └── browser native-messaging bridge
└── silo-tray
    └── desktop menu-bar/system-tray companion
```

## Request flow

```text
┌──────────────┐      native messaging      ┌─────────────────┐
│ Browser       │ ─────────────────────────▶ │ Native host     │
│ extension     │                            │ silo-native-host│
└──────────────┘                            └────────┬────────┘
                                                     │ TCP + token
                                                     ▼
                                            ┌─────────────────┐
                                            │ Broker          │
                                            │ silo-broker     │
                                            └────────┬────────┘
                                                     │ owns session
                                                     ▼
                                            ┌─────────────────┐
                                            │ Core            │
                                            │ encrypted vault │
                                            └─────────────────┘
```

The tray and CLI can also talk to the broker. The browser receives only an approved response, never the master password.

## Why `silo-core` is separate

The vault and cryptographic rules should not depend on Ratatui, GTK, browser messaging, or a desktop event loop. A small core API makes the most sensitive behavior easier to test:

```rust
pub fn save_vault(
    path: impl AsRef<Path>,
    vault: &Vault,
    password: &str,
) -> Result<(), Error>

pub fn load_vault(
    path: impl AsRef<Path>,
    password: &str,
) -> Result<Vault, Error>
```

The same vault implementation is used by the CLI, broker, migration tests, and recovery tests. There is no second encryption implementation hidden inside the UI.

## Why `silo-cli` is separate

The CLI owns user interaction:

```rust
match cli.command {
    Command::Init => init(&cli.vault)?,
    Command::Shell { timeout } => run_shell(&cli.vault, timeout)?,
    Command::Import { input, .. } => import_vault(&cli.vault, &input, ..)?,
    _ => {}
}
```

The TUI can evolve visually without changing protocol serialization or vault encryption. It also lets us test command behavior independently from the broker.

## Why `silo-broker` is separate

The broker is a session service, not a user-interface layer. Its responsibilities include:

- Keeping the decrypted vault only while unlocked.
- Applying inactivity timeout rules.
- Locking and clearing session state.
- Checking request tokens and expiration.
- Matching browser origins to vault entries.

This separation allows the tray to quit or restart without owning the encryption logic and allows a browser request to work without an open shell.

## Why `silo-protocol` is separate

The native host, broker, tray, and CLI must agree on message shapes. A shared crate prevents each process from inventing a slightly different JSON schema:

```rust
#[serde(tag = "type")]
pub enum Request {
    Status,
    GetLogin { url: String, entry_id: Option<String> },
    GetOtp { url: String },
    Lock,
    Unlock { password: SensitiveString },
}
```

The protocol crate contains framing, request IDs, expiration fields, and shared response types. It does not know how the vault is encrypted.

## Why `silo-native-host` is separate

Browsers launch native hosts using a browser-specific standard input/output protocol. The native host translates that protocol into Silo’s local broker request. Keeping it separate means browser packaging and host discovery do not enter the broker or core crates.

## Why `silo-tray` is separate

Tray APIs are platform-specific. A desktop companion needs macOS, Windows, and Linux integration, while the core vault should remain portable and headless. The tray starts a locked broker, reads status, and sends lock/unlock/open/quit actions.

## What a single crate would look like

One crate could contain all of this, but the boundaries would become implicit:

```text
one crate
├── crypto + vault
├── terminal UI
├── browser protocol
├── TCP broker
├── native host
└── platform tray code
```

That would make it easier for UI dependencies to reach sensitive code, harder to test the broker without the UI, and easier for two protocol implementations to drift apart.

The workspace still builds as one project. The crates are internal boundaries that make ownership and testing visible.

## The tradeoff

Separate crates add Cargo manifests, dependency wiring, and a little more ceremony. In exchange, Silo gets clearer security review surfaces:

```text
core       → cryptography and data correctness
protocol   → message correctness
broker     → session and authorization correctness
native host→ browser process boundary
tray       → desktop lifecycle
cli        → human interaction
```

That tradeoff is worth it for a password manager because the cost of a hidden responsibility is much higher than the cost of a few explicit crate boundaries.
