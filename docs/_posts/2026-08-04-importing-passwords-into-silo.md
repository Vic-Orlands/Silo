---
layout: post
title: "Importing 1Password, Authy, Bitwarden, KeePass, and browser passwords into Silo"
date: 2026-08-04
permalink: /importing-passwords-into-silo/
---

Migration should be a reviewable operation, not a long sequence of retyping passwords. Silo’s importer accepts local exports, previews them, validates entries, preserves supported TOTP setup data, regenerates IDs, and reports failures.

## The migration flow

```text
Export locally
      ↓
Silo detects or receives the format
      ↓
Dry-run preview
      ↓
Validate URLs, required fields, and TOTP
      ↓
Report duplicates and failed rows
      ↓
Apply after review
      ↓
Verify, then delete plaintext export
```

Preview first:

```bash
cargo run -p silo -- \
  --vault /tmp/silo-test/test.vault \
  import bitwarden.json \
  --dry-run
```

Apply only after checking the report:

```bash
cargo run -p silo -- \
  --vault /tmp/silo-test/test.vault \
  import bitwarden.json \
  --expect-count 42
```

`--expect-count` protects against importing an incomplete export. Exact duplicates are skipped, while different accounts on the same host remain separate.

## Supported export paths

| Source | Export to use | Silo format option |
| --- | --- | --- |
| Bitwarden | Unencrypted JSON or CSV | `bitwarden-json` or `csv` |
| 1Password | CSV export | `1password-csv` |
| KeePass / KeePassXC | CSV export | `keepass-csv` |
| Chrome / Firefox / Edge | Browser CSV export | `browser-csv` |
| Silo | Silo JSON export | `silo-json` |

Automatic detection is the default. Use an explicit format when a manager’s headers are ambiguous:

```bash
cargo run -p silo -- \
  --vault /tmp/silo-test/test.vault \
  import export.csv \
  --format keepass-csv \
  --dry-run
```

## How a row becomes an entry

The importer maps common column names into one internal candidate, then validates it before creating a fresh UUID:

```rust
let url = normalize_url(&candidate.url)?;

if candidate.password.is_empty() {
    return Err("missing password".into());
}

Ok(new_entry(
    candidate.name.trim().to_string(),
    url,
    candidate.username.trim().to_string(),
    candidate.email.trim().to_string(),
    candidate.password,
    totp,
))
```

Imported IDs are not trusted. Every accepted row receives a new UUID so an external manager cannot collide with an existing Silo identity.

## TOTP and Authy

The important distinction is between an authenticator app and the TOTP setup secret.

```text
website QR code / setup secret
          ↓
Authy calculates six-digit codes
          ↓
Silo can calculate the same codes if it receives the original secret
```

Silo accepts a raw Base32 setup secret or an `otpauth://` URI:

```bash
cargo run -p silo -- \
  --vault /tmp/silo-test/test.vault \
  set-totp github
```

Silo cannot directly import Authy’s encrypted internal application database. If Authy or the website allows the original secret/URI to be retrieved, import or paste it. Otherwise, temporarily re-enroll 2FA on the website and save the new setup secret in Silo.

Do not migrate the current six-digit code. The code changes every period; the setup secret is what must be preserved.

## What happens to malformed rows?

Rows are not silently discarded:

```text
Valid entries:   41
Duplicates:      2
Failed rows:     1
  Row 17: URL must use HTTP or HTTPS and include a host
```

This makes migration a controlled data operation rather than an optimistic bulk import.

## Plaintext export safety

Manager exports are usually plaintext. Keep them local, restrict their permissions, do not upload or commit them, verify the imported vault, then delete the export and empty the system trash. Encrypted Silo vault copies—not plaintext exports—are the long-term backups.
