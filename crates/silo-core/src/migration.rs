use crate::{inspect_totp, new_entry, Entry, Error, Vault};
use csv::StringRecord;
use serde_json::Value;
use std::collections::HashSet;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Auto,
    SiloJson,
    BitwardenJson,
    OnePasswordCsv,
    KeePassCsv,
    BrowserCsv,
    GenericCsv,
}

impl ImportFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "silo-json" | "silo" => Some(Self::SiloJson),
            "bitwarden-json" | "bitwarden" => Some(Self::BitwardenJson),
            "1password-csv" | "onepassword-csv" | "1password" => Some(Self::OnePasswordCsv),
            "keepass-csv" | "keepassxc-csv" | "keepass" => Some(Self::KeePassCsv),
            "browser-csv" | "browser" => Some(Self::BrowserCsv),
            "csv" | "generic-csv" => Some(Self::GenericCsv),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::SiloJson => "Silo JSON",
            Self::BitwardenJson => "Bitwarden JSON",
            Self::OnePasswordCsv => "1Password CSV",
            Self::KeePassCsv => "KeePass CSV",
            Self::BrowserCsv => "browser CSV",
            Self::GenericCsv => "generic CSV",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportIssue {
    pub row: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct MigrationPreview {
    pub format: ImportFormat,
    pub entries: Vec<Entry>,
    pub issues: Vec<ImportIssue>,
    pub duplicate_indices: Vec<usize>,
}

pub fn import_migration(
    bytes: &[u8],
    requested: ImportFormat,
    existing: &Vault,
) -> Result<MigrationPreview, Error> {
    let format = detect_format(bytes, requested)?;
    let candidates = match format {
        ImportFormat::SiloJson => parse_silo_json(bytes)?,
        ImportFormat::BitwardenJson => parse_bitwarden_json(bytes)?,
        ImportFormat::OnePasswordCsv
        | ImportFormat::KeePassCsv
        | ImportFormat::BrowserCsv
        | ImportFormat::GenericCsv => parse_csv(bytes, format)?,
        ImportFormat::Auto => unreachable!(),
    };
    let mut entries = Vec::new();
    let mut issues = Vec::new();
    for (row, candidate) in candidates.into_iter().enumerate() {
        match normalize_candidate(candidate) {
            Ok(entry) => entries.push(entry),
            Err(message) => issues.push(ImportIssue {
                row: row + 1,
                message,
            }),
        }
    }
    let duplicate_indices = duplicate_indices(&entries, existing);
    Ok(MigrationPreview {
        format,
        entries,
        issues,
        duplicate_indices,
    })
}

#[derive(Debug, Default)]
struct Candidate {
    name: String,
    url: String,
    username: String,
    email: String,
    password: String,
    totp: Option<String>,
}

fn detect_format(bytes: &[u8], requested: ImportFormat) -> Result<ImportFormat, Error> {
    if requested != ImportFormat::Auto {
        return Ok(requested);
    }
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        if value.get("items").is_some() {
            return Ok(ImportFormat::BitwardenJson);
        }
        if value.get("entries").is_some() && value.get("version").is_some() {
            return Ok(ImportFormat::SiloJson);
        }
        return Err(Error::Migration(
            "JSON format was not recognized; choose --format explicitly".into(),
        ));
    }
    let headers = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(bytes)
        .headers()
        .map_err(|error| Error::Migration(format!("could not read CSV headers: {error}")))?
        .iter()
        .map(normalize_header)
        .collect::<HashSet<_>>();
    if headers.contains("group") && headers.contains("title") {
        return Ok(ImportFormat::KeePassCsv);
    }
    if headers.contains("website") && headers.contains("extra") {
        return Ok(ImportFormat::OnePasswordCsv);
    }
    if headers.contains("url") && headers.contains("username") && headers.contains("password") {
        return Ok(ImportFormat::BrowserCsv);
    }
    Ok(ImportFormat::GenericCsv)
}

fn parse_silo_json(bytes: &[u8]) -> Result<Vec<Candidate>, Error> {
    let value: Value = serde_json::from_slice(bytes)?;
    let Some(version) = value.get("version").and_then(Value::as_u64) else {
        return Err(Error::Migration("Silo JSON is missing its version".into()));
    };
    if version != 1 {
        return Err(Error::UnsupportedFormat);
    }
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Migration("Silo JSON is missing entries".into()))?;
    Ok(entries
        .iter()
        .map(|entry| Candidate {
            name: text(entry, "name"),
            url: text(entry, "url"),
            username: text(entry, "username"),
            email: text(entry, "email"),
            password: text(entry, "password"),
            totp: optional_text(entry, "totp_secret"),
        })
        .collect())
}

fn parse_bitwarden_json(bytes: &[u8]) -> Result<Vec<Candidate>, Error> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value.get("encrypted").and_then(Value::as_bool) == Some(true) {
        return Err(Error::Migration(
            "Bitwarden export is encrypted; export an unencrypted JSON or CSV file locally, import it, then securely delete it".into(),
        ));
    }
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Migration("Bitwarden JSON is missing items".into()))?;
    Ok(items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_u64).unwrap_or(1) == 1)
        .map(|item| {
            let login = item.get("login").unwrap_or(&Value::Null);
            Candidate {
                name: text(item, "name"),
                url: login
                    .get("uris")
                    .and_then(Value::as_array)
                    .and_then(|uris| uris.first())
                    .map(|uri| text(uri, "uri"))
                    .unwrap_or_default(),
                username: text(login, "username"),
                email: String::new(),
                password: text(login, "password"),
                totp: optional_text(login, "totp"),
            }
        })
        .collect())
}

fn parse_csv(bytes: &[u8], format: ImportFormat) -> Result<Vec<Candidate>, Error> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|error| Error::Migration(format!("could not read CSV headers: {error}")))?
        .iter()
        .map(normalize_header)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for record in reader.records() {
        let record =
            record.map_err(|error| Error::Migration(format!("could not read CSV row: {error}")))?;
        candidates.push(candidate_from_record(&headers, &record, format));
    }
    Ok(candidates)
}

fn candidate_from_record(
    headers: &[String],
    record: &StringRecord,
    format: ImportFormat,
) -> Candidate {
    let field = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| headers.iter().position(|header| header == name))
            .and_then(|index| record.get(index))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let notes = field(&["notes", "extra"]);
    let otp = {
        let explicit = field(&["otp", "totp", "one-time-password", "one-time-code"]);
        if explicit.is_empty() {
            extract_otpauth(&notes).or_else(|| extract_secret_label(&notes))
        } else {
            Some(explicit)
        }
    };
    Candidate {
        name: field(&["name", "title", "login"]),
        url: field(&["url", "website", "uri", "login-url"]),
        username: field(&["username", "user-name", "user"]),
        email: field(&["email"]),
        password: field(&["password", "pass"]),
        totp: match format {
            ImportFormat::OnePasswordCsv
            | ImportFormat::KeePassCsv
            | ImportFormat::BrowserCsv
            | ImportFormat::GenericCsv => otp,
            ImportFormat::SiloJson | ImportFormat::BitwardenJson | ImportFormat::Auto => otp,
        },
    }
}

fn normalize_candidate(candidate: Candidate) -> Result<Entry, String> {
    if candidate.name.trim().is_empty() {
        return Err("missing name/title".into());
    }
    if candidate.username.trim().is_empty() {
        return Err("missing username".into());
    }
    if candidate.password.is_empty() {
        return Err("missing password".into());
    }
    let url = normalize_url(&candidate.url)?;
    let totp = match candidate.totp.filter(|value| !value.trim().is_empty()) {
        Some(value) => {
            inspect_totp(&value).map_err(|error| format!("invalid TOTP: {error}"))?;
            Some(value.trim().to_string())
        }
        None => None,
    };
    Ok(new_entry(
        candidate.name.trim().to_string(),
        url,
        candidate.username.trim().to_string(),
        candidate.email.trim().to_string(),
        candidate.password,
        totp,
    ))
}

fn normalize_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    let candidate = if value.contains("://") {
        value.to_string()
    } else {
        format!("https://{value}")
    };
    let parsed = Url::parse(&candidate).map_err(|_| "invalid URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("URL must use HTTP or HTTPS and include a host".into());
    }
    Ok(candidate)
}

fn duplicate_indices(entries: &[Entry], existing: &Vault) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let duplicate = existing
                .entries
                .iter()
                .chain(entries[..index].iter())
                .any(|other| {
                    other.username == entry.username
                        && other.password.as_str() == entry.password.as_str()
                        && same_host(&other.url, &entry.url)
                });
            duplicate.then_some(index)
        })
        .collect()
}

fn same_host(left: &str, right: &str) -> bool {
    Url::parse(left)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        == Url::parse(right)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
}

fn normalize_header(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn optional_text(value: &Value, key: &str) -> Option<String> {
    let text = text(value, key);
    (!text.is_empty()).then_some(text)
}

fn extract_otpauth(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .find(|part| part.starts_with("otpauth://"))
        .map(str::to_string)
}

fn extract_secret_label(value: &str) -> Option<String> {
    value.lines().find_map(|line| {
        let (label, secret) = line.split_once(':')?;
        label
            .trim()
            .to_ascii_lowercase()
            .contains("totp")
            .then_some(secret.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn imports_bitwarden_json_and_preserves_totp() {
        let bytes = br#"{
          "encrypted": false,
          "items": [{
            "type": 1,
            "name": "GitHub",
            "login": {
              "username": "alice@example.com",
              "password": "correct horse",
              "totp": "otpauth://totp/GitHub:alice?secret=JBSWY3DPEHPK3PXP",
              "uris": [{"uri": "https://github.com"}]
            }
          }]
        }"#;
        let preview = import_migration(bytes, ImportFormat::Auto, &Vault::new()).unwrap();
        assert_eq!(preview.format, ImportFormat::BitwardenJson);
        assert_eq!(preview.entries.len(), 1);
        assert!(preview.entries[0].totp_secret.is_some());
        assert_ne!(preview.entries[0].id, Uuid::nil());
    }

    #[test]
    fn imports_browser_csv_and_reports_invalid_rows() {
        let bytes = b"name,url,username,password\nGitHub,github.com,alice,secret\nMissing URL,,bob,secret\n";
        let preview = import_migration(bytes, ImportFormat::BrowserCsv, &Vault::new()).unwrap();
        assert_eq!(preview.entries.len(), 1);
        assert_eq!(preview.entries[0].url, "https://github.com");
        assert_eq!(preview.issues.len(), 1);
    }

    #[test]
    fn detects_duplicates_against_existing_vault() {
        let bytes = b"name,url,username,password\nGitHub,https://github.com,alice,secret\n";
        let mut existing = Vault::new();
        existing.add(new_entry(
            "GitHub".into(),
            "https://github.com".into(),
            "alice".into(),
            String::new(),
            "secret".into(),
            None,
        ));
        let preview = import_migration(bytes, ImportFormat::BrowserCsv, &existing).unwrap();
        assert_eq!(preview.duplicate_indices, vec![0]);
    }

    #[test]
    fn rejects_encrypted_bitwarden_export() {
        let bytes = br#"{"encrypted":true,"items":[]}"#;
        let error =
            import_migration(bytes, ImportFormat::BitwardenJson, &Vault::new()).unwrap_err();
        assert!(error.to_string().contains("encrypted"));
    }

    #[test]
    fn imports_onepassword_csv_and_extracts_totp_from_extra() {
        let bytes = b"title,website,username,password,extra\nGitHub,github.com,alice,secret, TOTP: JBSWY3DPEHPK3PXP\n";
        let preview = import_migration(bytes, ImportFormat::Auto, &Vault::new()).unwrap();
        assert_eq!(preview.format, ImportFormat::OnePasswordCsv);
        assert_eq!(preview.entries.len(), 1);
        assert_eq!(
            preview.entries[0].totp_secret.as_ref().unwrap().as_str(),
            "JBSWY3DPEHPK3PXP"
        );
    }

    #[test]
    fn imports_keepass_csv_and_regenerates_duplicate_source_ids() {
        let bytes = b"Group,Title,Username,Password,URL,Notes\nWeb,GitHub,alice,secret,https://github.com,\n";
        let preview = import_migration(bytes, ImportFormat::Auto, &Vault::new()).unwrap();
        assert_eq!(preview.format, ImportFormat::KeePassCsv);
        assert_eq!(preview.entries.len(), 1);
        assert_ne!(preview.entries[0].id, Uuid::nil());
    }

    #[test]
    fn detects_duplicates_within_an_import_but_keeps_other_accounts() {
        let bytes = b"name,url,username,password\nGitHub,github.com,alice,secret\nGitHub copy,github.com,alice,secret\nGitHub other,github.com,bob,secret\n";
        let preview = import_migration(bytes, ImportFormat::BrowserCsv, &Vault::new()).unwrap();
        assert_eq!(preview.entries.len(), 3);
        assert_eq!(preview.duplicate_indices, vec![1]);
    }
}
