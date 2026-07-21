# Silo fuzzing

Install cargo-fuzz, then run these targets from this directory:

```bash
cargo fuzz run totp_input
cargo fuzz run vault_file
```

The targets exercise untrusted TOTP strings and vault bytes. The vault target intentionally uses a fixed temporary path because fuzzing runs in an isolated process; do not run it against a real vault.
