---
layout: post
title: "What Silo does with your master password"
date: 2026-08-02
permalink: /secret-lifecycle-hardening/
---

When we started Silo, the central question was simple: can a local password manager keep secrets useful without keeping them around longer than necessary?

The answer is not “never place a password in memory.” That is impossible if the application must use the password. The better goal is to control the password’s lifetime, minimize copies, authenticate every decrypted result, and be honest about what the operating system can still expose.

## The starting point

Silo already encrypted vaults at rest with Argon2id-derived keys and XChaCha20-Poly1305. That gave us the right cryptographic direction, but the application around the vault still had avoidable exposure:

- Cloneable protocol messages could copy sensitive values.
- `Debug` output could accidentally reveal secrets.
- `SecretString` exposed its inner `String` publicly.
- Temporary buffers were cleared, but memory-page locking was not attempted.
- The boundaries between the CLI, broker, native host, browser, and clipboard were not documented together.

None of these findings meant that Argon2id was insecure. They were application-lifecycle problems around a sound primitive.

## What changed

We made the secret lifecycle narrower and more explicit.

### Redacted diagnostics

Sensitive types no longer print their contents through `Debug`. This matters because debug output often reaches test failures, logs, crash reports, or developer tooling.

The implementation deliberately gives the wrapper a safe diagnostic representation:

```rust
impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SecretString")
            .field(&"[REDACTED]")
            .finish()
    }
}
```

The secret is still available through a narrow borrow when cryptographic code needs it:

```rust
let key = derive_key(password.as_bytes(), &salt, 64 * 1024, 3, 1)?;
```

### Fewer protocol copies

The native host now moves a request into its broker envelope instead of cloning the request first. This is particularly useful for unlock and save-login messages.

### Private secret ownership

`SecretString` now owns its internal storage privately. Callers can borrow its contents when an operation needs them, but they cannot reach into the underlying string directly.

### Zeroization remains part of the design

Passwords, derived keys, decrypted buffers, broker responses, and clipboard values are cleared when the owning scope ends. The `zeroize` crate uses mechanisms intended to prevent the compiler from deleting the clearing operation.

### Best-effort memory locking

Silo now attempts `mlock` on Unix and `VirtualLock` on Windows for short-lived sensitive buffers. This can reduce the chance of sensitive pages being written to swap, but it is not guaranteed: operating systems can reject the request due to quotas, permissions, sandboxing, or policy.

The important design decision is to treat failure honestly. If page locking is unavailable, Silo continues with zeroization and normal operating-system protections; it does not claim that the memory is locked.

The guard owns no secret data. It only remembers the address and length, locks the pages during the scope, and unlocks them when the scope ends:

```rust
let _password_lock = memory::Locked::new(password.as_bytes());
let mut key = Zeroizing::new([0u8; 32]);
let _key_lock = memory::Locked::new(&key[..]);

argon2.hash_password_into(password, salt, key.as_mut())?;
```

## What happens during save and load?

On save:

```text
master password + random salt
        ↓
Argon2id
        ↓
derived encryption key
        ↓
XChaCha20-Poly1305 encrypts vault
```

On load, the same derivation happens again with the stored salt. The resulting key attempts authenticated decryption. There is no stored master-password hash being compared. A wrong password derives the wrong key, and the authentication tag rejects it.

The save path is similarly explicit:

```rust
let plaintext = Zeroizing::new(serde_json::to_vec(vault)?);
let key = derive_key(password.as_bytes(), &salt, 64 * 1024, 3, 1)?;
let cipher = XChaCha20Poly1305::new((&*key).into());
let ciphertext = cipher.encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())?;
```

The password is therefore present in memory during both derivation paths. Zeroization happens after the value is no longer needed. It cannot protect a process that a privileged attacker is actively inspecting while it is unlocked.

This is also why bcrypt would not solve the memory concern. bcrypt receives a password in memory too, while Argon2id is a better fit for this vault because it is memory-hard and avoids bcrypt’s traditional 72-byte input limit.

## Why we did not promise perfect memory security

Application code cannot fully control:

- Temporary copies created by libraries or operating-system boundaries.
- CPU registers and compiler-generated stack movement.
- A privileged debugger inspecting the process.
- Crash dumps or swap behavior outside the application’s control.
- Other processes with sufficient operating-system privileges.

Zeroization is valuable cleanup, not magic erasure. Page locking is valuable defense in depth, not a guarantee.

## What we tested

The hardening work is covered by workspace tests and Clippy checks, including:

- Vault encryption and wrong-password rejection.
- Protocol serialization and broker behavior.
- Secret debug redaction.
- Broker lock and timeout behavior.
- Native-host recovery.
- Browser smoke tests.
- Cross-platform packaging checks.

## What remains

Silo still needs an independent security audit before it should hold irreplaceable production credentials. We also need platform-specific sleep/wake tests, a formal review of allocator and library copies, a carefully designed OS-keychain or hardware-backed unlock mode, reproducible signed releases, and broader browser/broker threat-model testing.

The result is a more disciplined local-first password manager—not one that claims to defeat a compromised operating system.
