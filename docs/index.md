---
layout: default
title: Silo
description: A local-first password manager for the terminal, desktop, and browser.
---

<p class="eyebrow">LOCAL-FIRST CREDENTIALS</p>

# Silo

Your passwords stay in an encrypted vault on your computer. Silo brings the same vault to the terminal, a background desktop session, and an explicit browser bridge.

<div class="hero-actions">
  <a class="button" href="https://github.com/Vic-Orlands/Silo">View on GitHub</a>
  <a class="quiet-link" href="https://github.com/Vic-Orlands/Silo#start-here">Read the setup guide</a>
</div>

## What Silo does

- Stores the vault locally with Argon2id key derivation and XChaCha20-Poly1305 encryption.
- Provides a focused terminal workspace for creating, searching, editing, and copying credentials.
- Keeps the browser connection local through a broker and native-messaging host.
- Supports TOTP setup secrets and standard `otpauth://` URIs.
- Imports Bitwarden JSON, 1Password CSV, KeePass/KeePassXC CSV, browser CSV, generic CSV, and Silo JSON.

## Start locally

```bash
cargo run -p silo -- init
cargo run -p silo -- shell
```

For browser access, install the background broker and native host using the instructions in the repository README.

## Migration safety

Preview an import before changing the vault:

```bash
cargo run -p silo -- \
  --vault /path/to/silo.vault \
  import export.json \
  --dry-run
```

Silo validates imported fields, regenerates entry IDs, reports failed rows, preserves supported TOTP data, and skips exact duplicates. Plaintext exports should be created locally, verified, and securely deleted after migration.

## Security position

Silo is local-first, not cloud-synchronized. The master password is used in memory to derive an encryption key and is cleared after use where the runtime can control its lifetime. The browser never receives the master password; it only receives explicitly approved field values or TOTP codes.

Silo is not yet a replacement for an independently audited production password manager. Review [SECURITY.md](https://github.com/Vic-Orlands/Silo/blob/main/SECURITY.md) before storing irreplaceable credentials.

## Project status

Silo is under active development. The source, tests, migration notes, packaging scripts, and issue tracker are available on [GitHub](https://github.com/Vic-Orlands/Silo).

## Engineering notes

- [What Silo does with your master password]({{ "/secret-lifecycle-hardening/" | relative_url }})
- [Building Silo Tray]({{ "/building-silo-tray/" | relative_url }})
- [Migrating passwords and TOTP accounts into Silo]({{ "/importing-passwords-into-silo/" | relative_url }})
- [Silo architecture explained]({{ "/silo-architecture/" | relative_url }})
