---
layout: default
title: Security hardening
---

# Security hardening in Silo

This note records why Silo uses Argon2id, what changed in the secret lifecycle, and what the implementation still cannot promise.

## Before

The first vault flow was already encrypted at rest with Argon2id and XChaCha20-Poly1305, but the surrounding application had wider secret lifetimes than necessary:

- Some protocol values could be cloned because the whole request type was cloneable.
- Sensitive types could derive `Debug`, making accidental logging dangerous.
- The inner value of `SecretString` was public.
- Temporary password and decrypted-vault buffers were zeroized, but memory-page locking was not attempted.
- The security boundary between vault encryption, broker transport, clipboard use, and browser responses was not documented in one place.

These were lifecycle and operational risks, not evidence that Argon2id itself was unsafe.

## After

Silo now:

- Redacts secret values from `Debug` output.
- Keeps `SecretString` internals private.
- Uses zeroizing wrappers for temporary password, key, response, clipboard, and decrypted-data values where the owning code controls their lifetime.
- Removes the native-host request clone path, so an unlock request is moved into its envelope instead of copied there.
- Attempts best-effort `mlock` on Unix and `VirtualLock` on Windows around short-lived password, derived-key, plaintext, and decrypted-data buffers.
- Clears broker session state on lock and timeout.
- Keeps passwords out of command-line arguments and broker error messages.
- Tests wrong-password behavior, protocol handling, broker recovery, and secret redaction.

Page locking is deliberately best-effort. Operating systems can reject the request because of quotas, permissions, memory pressure, sandboxing, or platform policy. Silo continues safely if locking is unavailable; it does not claim that memory is locked when the operating system refused it.

## Why Argon2id remains

The master password is not stored as a comparison hash. On both save and load, Silo derives a key from:

```text
master password + stored random salt → Argon2id → encryption key
```

That key encrypts or decrypts the vault using authenticated XChaCha20-Poly1305 encryption. A wrong password produces the wrong key, so authentication fails.

The password must exist briefly in memory while Argon2id runs. Zeroization protects the memory after the value is no longer needed; it cannot stop a privileged attacker from inspecting a live process during derivation. bcrypt has the same plaintext-in-memory property and is less suitable for this vault use because of its traditional 72-byte input limit and lack of Argon2id’s memory-hard design.

## Before migrating important credentials

Use full-disk encryption, keep the operating-system account protected, avoid debugging or logging a live Silo process, and maintain more than one encrypted vault backup. Do not treat a plaintext export as a backup.

## Remaining limitations

Silo still needs:

- Independent third-party security review.
- Platform-specific verification of page-lock behavior and sleep/wake behavior.
- A formal audit of all compiler/library copies and allocator behavior.
- A reviewed OS-keychain or hardware-backed vault-key design.
- Production signing and reproducible release verification.
- Additional browser and broker threat-model testing.

No application-level zeroization design can protect against a fully privileged attacker controlling the operating system or inspecting the process while it is unlocked.
