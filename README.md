# Silo

Silo is a local-first password manager from scratch. The command-line interface and local browser bridge are the primary product surfaces. You can also use the Shell for better UX interactivity.

https://github.com/user-attachments/assets/c8749aa5-8d67-48ef-972e-2d66a75aac59

## Start here

Create a local encrypted vault:

```bash
cargo run -p silo -- init
```

Add a login interactively:

```bash
cargo run -p silo -- add github --url https://github.com --username you@example.com
```

The command asks for the login password and then asks whether you want to save a TOTP secret. You can also provide it directly:

```bash
cargo run -p silo -- add github \
  --url https://github.com \
  --username you@example.com \
  --totp-secret JBSWY3DPEHPK3PXP
```

## CLI design

Every command unlocks the local vault, performs one focused operation, saves if necessary, and exits. This is intentionally simple while we learn the system. A later session mode can keep the vault unlocked for several commands.

```text
silo init                         Create an encrypted vault
silo add <name>                   Add a login; missing details are prompted for
silo list                         List names, usernames, and URLs
silo show <name>                  Show metadata without displaying the password
silo get <name> username          Print a username
silo get <name> password          Print a password explicitly
silo get <name> url               Print a URL
silo otp <name>                   Print the current six-digit TOTP code
silo otp-check <name>             Validate TOTP setup and explain its configuration
silo set-totp <name>              Add, replace, or clear a TOTP secret
silo edit <name>                  Change metadata or use --password to change password
silo remove <name>                Delete an entry after confirmation
silo remove <name> --yes          Delete without confirmation
silo generate                     Generate a password without saving it
silo shell                        Unlock once and work interactively
silo broker                       Run the local browser session broker
silo broker --background          Run a locked broker without a terminal session
silo unlock                       Unlock the background broker
silo lock                         Lock the background broker
silo status                       Show broker state
silo copy <name> password          Copy a secret and clear it later
silo export <file>                Export plaintext JSON deliberately
silo import <file>                Import plaintext JSON into the vault
```

`add` accepts `--url`, `--username`, `--email`, `--password`, `--password-file`, and `--totp-secret`. Silo only prompts for values you omit. Prefer `--password-file` for scripts because `--password` can be exposed in shell history or process listings.

The shell is a full-screen terminal workspace with a quiet editorial language: near-black canvas, whitespace-led hierarchy, a single navigation rail, and emerald reserved for live/success state. Set your terminal to Monaspace Radon for the intended feel; Silo reserves italic styling for keyboard callouts. Unlock and create flows use short checkmark / progress ceremonies. Default inactivity timeout is 15 minutes:

```bash
cargo run -p silo -- shell
cargo run -p silo -- shell --timeout 300
```

For browser use without keeping a shell open, install the broker as a login service. It starts locked, keeps running in the background, and holds decrypted vault data only after you unlock it:

```bash
cargo install --path crates/silo-cli --force
SILO_CLI_BIN="$HOME/.cargo/bin/silo" \
  sh scripts/install-broker.sh /tmp/silo-test/test.vault
```

The macOS LaunchAgent or Linux user service starts the broker automatically. On Windows, use `scripts/install-broker.ps1`. Unlock it when needed:

```bash
silo unlock
silo status
```

The browser extension's **Unlock Silo** action opens a short-lived unlock session. It does not keep a management shell running. The shell remains available for full vault management and also starts an unlocked broker for that interactive session:

```bash
cargo run -p silo -- --vault /tmp/silo-test/test.vault shell --timeout 900
```

The broker owns the unlocked vault session, locks after the shared timeout, clears the decrypted vault and master password on lock, and removes its state when the owning process exits. The browser extension only receives explicitly approved login/TOTP results; the master password is not entered into or stored by the browser extension.

Inside the shell:

```text
↑ / ↓ or j / k       Select an entry
enter                 Open entry details
→                     Open details and navigate overview fields
← / esc               Leave field copy mode (or close details)
/                     Search entries (also: click the search input)
n                     Create a login
e                     Edit the selected login
d                     Delete after confirmation
c                     Copy password; or copy the marked overview field
o                     Generate and copy TOTP; clears after 20 seconds
Ctrl-U                Clear the active input
?                     Keys & how to use Silo (scrollable)
Ctrl-S                Save the current form
x                     Reveal or hide the password in a form
q                     Quit and lock
```

Click an authentication to select it, or click a detail field to mark it for copy. Forms support mid-string editing with arrow keys / mouse click. Search text wraps and the input grows with content.

Clipboard copy runs in the background so the workspace stays usable, then clears the clipboard after 20 seconds only if another application has not replaced the copied value. Export is deliberately explicit because the output is plaintext JSON and must be protected or deleted after use. Saves are atomic and retain the previous vault as a `.bak` file.

Use another vault file with `--vault`:

```bash
cargo run -p silo -- --vault old-silo.vault list
```

The previous `UZOPASS` file header is still accepted for compatibility. Newly saved vaults use the `SILO` header with recorded Argon2id parameters. Your existing `uzopass.vault` file is not moved or deleted; pass it explicitly with `--vault uzopass.vault` while transitioning.

## Why TOTP failed before

`otp github` does not create a TOTP secret. It calculates a code from a secret already stored on the GitHub entry. The old CLI made it easy to create an entry without that secret. The new flow makes the setup visible:

```bash
cargo run -p silo -- set-totp github
```

Paste the secret shown by the website. Then:

```bash
cargo run -p silo -- otp github
```

The TOTP implementation currently supports the common six-digit, 30-second HMAC-SHA1 format. It accepts either a raw Base32 setup secret or a standard `otpauth://` URI copied from a QR-code tool. The value must be the setup secret, not the six-digit code currently displayed by an authenticator app.

When a code does not work, use the diagnostic command:

```bash
cargo run -p silo -- --vault silo.vault otp-check github
```

It reports the source format, algorithm, digit count, period, decoded byte length, current code, and time remaining without printing the secret.

## Code map for learning

- `crates/silo-core`: data structures, encryption, vault file format, URL matching, and TOTP.
- `crates/silo-cli`: command parsing, prompts, and user-facing behavior.
- `crates/silo-broker`: unlocked local session, timeout, lock, and browser request policy.
- `crates/silo-protocol`: versioned JSON requests, responses, broker state, and native-messaging frames shared by the broker and host.
- `crates/silo-native-host`: native messaging bridge process.
- `extension`: browser bridge with explicit popup actions for login and one-time-code filling.

## Browser bridge installation

Build the native host, install the extension temporarily in the browser, then use its extension ID:

```bash
cargo build -p silo-native-host
sh scripts/install-native-host.sh YOUR_EXTENSION_ID
```

On Windows, run `scripts/install-native-host.ps1` from PowerShell. The native host is a thin bridge to the local broker and can start a locked broker if the login service is not already running. For a release build, set `SILO_NATIVE_HOST_BIN=target/release/silo-native-host` before running the installer.

The installer records the Silo CLI path for the extension's `Open Silo` action. Set `SILO_CLI_BIN` when the CLI is not available through the browser's `PATH`:

```bash
SILO_NATIVE_HOST_BIN="$PWD/target/debug/silo-native-host" \
SILO_CLI_BIN="$(command -v silo)" \
SILO_VAULT="/tmp/silo-test/test.vault" \
sh scripts/install-native-host.sh YOUR_EXTENSION_ID chrome
```

Rust concepts to notice:

- `struct` models a vault entry.
- `enum` models commands and field choices.
- `Result<T, E>` makes failure explicit.
- `Option<T>` represents optional TOTP data.
- `&mut` gives a function permission to edit an entry.
- `derive` generates repetitive implementations such as CLI parsing and serialization.

## Verification

```bash
cargo fmt --all
cargo test --workspace
cargo run -p silo -- --help
```

The same checks are available through `sh scripts/verify.sh`. Packaging checks are run with `sh scripts/test-packaging.sh`. Dependency audits run in CI with `cargo-audit`; fuzz targets live in `fuzz/` and require the nightly toolchain:

```bash
cargo install cargo-fuzz
rustup toolchain install nightly
cd fuzz
cargo +nightly fuzz run totp_input -- -max_total_time=60
cargo +nightly fuzz run vault_file -- -max_total_time=60
```

Tagged releases are built for Linux, macOS, and Windows. The release workflow signs each SHA-256 checksum with Cosign. Configure `COSIGN_PRIVATE_KEY` and `COSIGN_PASSWORD` repository secrets, then verify release signatures with Silo's published public key.

Browser checks require Node.js and Playwright:

```bash
sh scripts/browser-smoke.sh
```

The smoke test loads the unpacked extension in Chromium and verifies the popup search field and explicit approval flow. The Rust integration test starts a temporary broker and native host and verifies login, TOTP, save-login, locking, expiry, domain matching, and token rejection.

Silo is pre-release software and has not undergone an independent security audit. Do not use it as your only password manager for important accounts until memory handling, backups, lock behavior, browser integration, update signing, and security testing are complete.
