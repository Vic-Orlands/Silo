# Silo fuzzing

Install cargo-fuzz and the nightly Rust toolchain, then run these targets from this directory. `cargo-fuzz` enables sanitizer instrumentation with unstable compiler flags, so stable Rust cannot build the targets:

```bash
cargo install cargo-fuzz
rustup toolchain install nightly
cargo +nightly fuzz run totp_input
cargo +nightly fuzz run vault_file
```

The targets exercise untrusted TOTP strings and vault bytes. The vault target intentionally uses a fixed temporary path because fuzzing runs in an isolated process; do not run it against a real vault.
