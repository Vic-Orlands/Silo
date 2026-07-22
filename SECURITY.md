# Silo security model

Silo is an educational local-first password manager. It is not a replacement for an independently audited password manager.

## Trust boundaries

- The encrypted vault is the durable secret store.
- `silo broker` is the unlocked local session owner.
- The browser extension can request only the current page's matching login or TOTP.
- The native host forwards browser requests to the broker over loopback TCP.
- The broker and native host use a versioned, length-delimited protocol with a per-session token.
- The broker state file contains a random per-session token and is restricted to the current user on Unix systems.

## Important guarantees

- The browser extension never stores the master password.
- The native host never prompts for or stores the master password.
- Browser filling is explicit; page load does not fill credentials.
- HTTP pages do not match HTTPS entries.
- Hostnames must match exactly or be a real subdomain.
- Vault writes are authenticated, atomic, and retain a backup.
- Broker sessions lock after inactivity and support manual `lock`.
- Vault KDF parameters are bounded before Argon2id work begins, and malformed/truncated files fail closed.

## Known limitations

- Loopback TCP is protected by the session token, not by transport encryption.
- A local process running as the same user may be able to inspect or interfere with the session.
- Browser DOM filling still places secrets into the page and browser process memory.
- Plaintext JSON exports must be protected and deleted after use.
- Release checksums are signed with Cosign in the tag workflow; consumers must verify the published signature with Silo's public key.
- `cargo audit` and `cargo fuzz` must be installed and run separately; they are not bundled with Rust.

Report suspected vulnerabilities privately before opening a public issue.
