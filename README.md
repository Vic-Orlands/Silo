# Silo


Silo is a local-first password manager built in Rust. It keeps the vault on your machine and makes every release of a secret an explicit action.


[Download the latest release](https://github.com/Vic-Orlands/Silo/releases/latest) · [Architecture](https://github.com/Vic-Orlands/Silo/blob/main/docs/_posts/2026-08-05-silo-architecture.md) · [Security hardening](https://github.com/Vic-Orlands/Silo/blob/main/docs/security-hardening.md) · [Website](https://silo-ruddy.vercel.app/) · [Marketing site repository](https://github.com/Vic-Orlands/silo-site)


![Silo terminal workspace](https://github.com/user-attachments/assets/c8749aa5-8d67-48ef-972e-2d66a75aac59)


## Why Silo exists


Many password managers begin with an account and synchronization service. Silo begins with a local encrypted file. The CLI, terminal workspace, desktop tray, and browser bridge all operate around that vault without requiring a cloud account or sync layer.
