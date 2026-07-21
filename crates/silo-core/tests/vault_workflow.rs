use silo_core::{load_vault, new_entry, save_vault, Vault};
use std::fs;
use uuid::Uuid;

#[test]
fn encrypted_vault_supports_add_find_and_remove() {
    let path = std::env::temp_dir().join(format!("silo-integration-{}.vault", Uuid::new_v4()));
    let mut vault = Vault::new();
    vault.add(new_entry(
        "GitHub".into(),
        "https://github.com".into(),
        "alice".into(),
        "alice@example.com".into(),
        "correct horse battery staple".into(),
        Some("JBSWY3DPEHPK3PXP".into()),
    ));

    save_vault(&path, &vault, "master password").unwrap();
    let mut loaded = load_vault(&path, "master password").unwrap();
    assert_eq!(loaded.find("github").unwrap().username, "alice");
    assert_eq!(loaded.find("github").unwrap().email, "alice@example.com");
    assert_eq!(
        loaded.find("github").unwrap().password.as_str(),
        "correct horse battery staple"
    );
    assert!(loaded.find_for_url("https://github.com/login").is_some());
    assert!(loaded
        .find_for_url("https://gist.github.com/example")
        .is_some());
    assert!(loaded.find_for_url("http://github.com/login").is_none());
    assert!(loaded
        .find_for_url("https://github.com.evil.example")
        .is_none());
    assert!(loaded.remove("github").is_some());
    assert!(loaded.entries.is_empty());
    fs::remove_file(path).unwrap();
}
