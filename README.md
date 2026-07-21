# Silo

Silo is a learning project for building a local-first password manager from scratch. The command-line interface is the primary product. The browser bridge exists only as an early experiment and is not the focus yet.

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
- `crates/silo-native-host`: native messaging bridge process.
- `extension`: browser bridge with explicit popup actions for login and one-time-code filling.

## Browser bridge installation

Build the native host, install the extension temporarily in the browser, then use its extension ID:

```bash
cargo build -p silo-native-host
SILO_VAULT_PATH="$HOME/.local/share/silo/silo.vault" \
  sh scripts/install-native-host.sh YOUR_EXTENSION_ID
```

On Windows, run `scripts/install-native-host.ps1` from PowerShell. The native host starts locked, keeps one browser connection alive for its process lifetime, and is unlocked from the extension popup for the current session. The popup clears the password field immediately after sending it; it is not persisted by the extension.

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

This remains an educational prototype, not an audited password manager. Do not use it as your only password manager for important accounts until memory handling, backups, lock behavior, browser integration, update signing, and security testing are complete.
