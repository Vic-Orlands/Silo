use argon2::{Algorithm, Argon2, Params, Version};
use base32::{decode as decode_base32, Alphabet};
use base64::{engine::general_purpose, Engine as _};
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use prost::Message;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha1::Sha1;
use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;
use url::Url;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

mod memory;
mod migration;
pub use migration::{import_migration, ImportFormat, ImportIssue, MigrationPreview};

const MAGIC: &[u8; 8] = b"SILO\0\0\0\0";
const LEGACY_MAGIC: &[u8; 8] = b"UZOPASS\0";
const FORMAT_VERSION: u8 = 2;
const LEGACY_FORMAT_VERSION: u8 = 1;
const SALT_LENGTH: usize = 16;
const NONCE_LENGTH: usize = 24;
const MAX_MEMORY_KIB: u32 = 1024 * 1024;
const MAX_ITERATIONS: u32 = 100;
const MAX_PARALLELISM: u32 = 64;
const EXPORT_VERSION: u8 = 1;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not read vault: {0}")]
    Io(#[from] std::io::Error),
    #[error("vault has an unsupported format")]
    UnsupportedFormat,
    #[error("vault password is incorrect or the file is damaged")]
    InvalidPasswordOrVault,
    #[error("could not encode vault data: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("invalid TOTP secret")]
    InvalidTotpSecret,
    #[error("TOTP secret uses an unsupported algorithm")]
    UnsupportedTotpAlgorithm,
    #[error("TOTP secret has an invalid number of digits")]
    InvalidTotpDigits,
    #[error("TOTP URI uses an unsupported period; Silo currently requires 30 seconds")]
    UnsupportedTotpPeriod,
    #[error("Google Authenticator migration QR data is not an individual TOTP secret; use the account's setup secret or export accounts individually")]
    UnsupportedTotpMigration,
    #[error("TOTP secret is valid, but its configuration is unsupported: {0}")]
    UnsupportedTotpConfiguration(String),
    #[error("migration import failed: {0}")]
    Migration(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpMetadata {
    pub source: &'static str,
    pub algorithm: &'static str,
    pub digits: u8,
    pub period: u64,
    pub secret_bytes: usize,
}

#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SecretString(String);

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SecretString")
            .field(&"[REDACTED]")
            .finish()
    }
}

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub username: String,
    #[serde(default)]
    pub email: String,
    pub password: SecretString,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp_secret: Option<SecretString>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Vault {
    pub entries: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportDocument {
    pub version: u8,
    pub entries: Vec<Entry>,
}

impl Vault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    pub fn find_mut(&mut self, query: &str) -> Option<&mut Entry> {
        let query = query.to_lowercase();
        self.entries.iter_mut().find(|entry| {
            entry.name.to_lowercase() == query
                || entry.url.to_lowercase().contains(&query)
                || entry.username.to_lowercase() == query
                || entry.email.to_lowercase() == query
        })
    }

    pub fn remove(&mut self, query: &str) -> Option<Entry> {
        let query = query.to_lowercase();
        let index = self.entries.iter().position(|entry| {
            entry.name.to_lowercase() == query
                || entry.url.to_lowercase().contains(&query)
                || entry.username.to_lowercase() == query
                || entry.email.to_lowercase() == query
        })?;
        Some(self.entries.remove(index))
    }

    pub fn find(&self, query: &str) -> Option<&Entry> {
        let query = query.to_lowercase();
        self.entries.iter().find(|entry| {
            entry.name.to_lowercase() == query
                || entry.url.to_lowercase().contains(&query)
                || entry.username.to_lowercase() == query
                || entry.email.to_lowercase() == query
        })
    }

    pub fn find_for_url(&self, current_url: &str) -> Option<&Entry> {
        self.find_all_for_url(current_url).into_iter().next()
    }

    pub fn find_all_for_url(&self, current_url: &str) -> Vec<&Entry> {
        let Some(current) = Url::parse(current_url).ok() else {
            return Vec::new();
        };
        let current_scheme = current.scheme().to_ascii_lowercase();
        let Some(current_host) = current.host_str().map(str::to_lowercase) else {
            return Vec::new();
        };
        let current_port = current.port_or_known_default();
        self.entries
            .iter()
            .filter(|entry| {
                let Ok(entry_url) = Url::parse(&entry.url) else {
                    return false;
                };
                let Some(entry_host) = entry_url.host_str().map(str::to_lowercase) else {
                    return false;
                };
                let scheme_matches = current_scheme == entry_url.scheme().to_ascii_lowercase();
                let port_matches = current_port == entry_url.port_or_known_default();
                scheme_matches
                    && port_matches
                    && (current_host == entry_host
                        || current_host.ends_with(&format!(".{entry_host}")))
            })
            .collect()
    }
}

pub fn save_vault(path: impl AsRef<Path>, vault: &Vault, password: &str) -> Result<(), Error> {
    let plaintext = Zeroizing::new(serde_json::to_vec(vault)?);
    let _plaintext_lock = memory::Locked::new(&plaintext);
    let mut salt = [0u8; SALT_LENGTH];
    let mut nonce = [0u8; NONCE_LENGTH];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let key = derive_key(password.as_bytes(), &salt, 64 * 1024, 3, 1)?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| Error::InvalidPasswordOrVault)?;

    let mut bytes =
        Vec::with_capacity(MAGIC.len() + 1 + 12 + SALT_LENGTH + NONCE_LENGTH + ciphertext.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(FORMAT_VERSION);
    bytes.extend_from_slice(&(64 * 1024u32).to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&salt);
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&ciphertext);

    atomic_write(path.as_ref(), &bytes)?;
    Ok(())
}

pub fn load_vault(path: impl AsRef<Path>, password: &str) -> Result<Vault, Error> {
    let bytes = fs::read(path)?;
    if bytes.len() < MAGIC.len() + 1
        || (&bytes[..MAGIC.len()] != MAGIC && &bytes[..MAGIC.len()] != LEGACY_MAGIC)
    {
        return Err(Error::UnsupportedFormat);
    }
    let version = bytes[MAGIC.len()];
    let (salt_start, memory_kib, iterations, parallelism) = match version {
        LEGACY_FORMAT_VERSION => (MAGIC.len() + 1, 64 * 1024, 3, 1),
        FORMAT_VERSION => {
            let params_start = MAGIC.len() + 1;
            let params_end = params_start + 12;
            if bytes.len() < params_end {
                return Err(Error::UnsupportedFormat);
            }
            (
                params_end,
                u32::from_le_bytes(bytes[params_start..params_start + 4].try_into().unwrap()),
                u32::from_le_bytes(
                    bytes[params_start + 4..params_start + 8]
                        .try_into()
                        .unwrap(),
                ),
                u32::from_le_bytes(bytes[params_start + 8..params_end].try_into().unwrap()),
            )
        }
        _ => return Err(Error::UnsupportedFormat),
    };
    if !(16 * 1024..=MAX_MEMORY_KIB).contains(&memory_kib)
        || iterations == 0
        || iterations > MAX_ITERATIONS
        || parallelism == 0
        || parallelism > MAX_PARALLELISM
    {
        return Err(Error::UnsupportedFormat);
    }

    let nonce_start = salt_start + SALT_LENGTH;
    let ciphertext_start = nonce_start + NONCE_LENGTH;
    if bytes.len() < ciphertext_start + 16 {
        return Err(Error::UnsupportedFormat);
    }
    let salt = &bytes[salt_start..nonce_start];
    let nonce = &bytes[nonce_start..ciphertext_start];
    let ciphertext = &bytes[ciphertext_start..];

    let key = derive_key(
        password.as_bytes(),
        salt,
        memory_kib,
        iterations,
        parallelism,
    )?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .map_err(|_| Error::InvalidPasswordOrVault)?,
    );
    let _plaintext_lock = memory::Locked::new(&plaintext);

    Ok(serde_json::from_slice(&plaintext)?)
}

pub fn new_entry(
    name: String,
    url: String,
    username: String,
    email: String,
    password: String,
    totp_secret: Option<String>,
) -> Entry {
    Entry {
        id: Uuid::new_v4(),
        name,
        url,
        username,
        email,
        password: SecretString::new(password),
        totp_secret: totp_secret.map(SecretString::new),
    }
}

pub fn export_json(vault: &Vault) -> Result<Vec<u8>, Error> {
    Ok(serde_json::to_vec_pretty(&ExportDocument {
        version: EXPORT_VERSION,
        entries: vault.entries.clone(),
    })?)
}

pub fn import_json(bytes: &[u8]) -> Result<Vault, Error> {
    let document: ExportDocument = serde_json::from_slice(bytes)?;
    if document.version != EXPORT_VERSION {
        return Err(Error::UnsupportedFormat);
    }
    Ok(Vault {
        entries: document.entries,
    })
}

pub fn generate_totp(secret: &str, timestamp: u64) -> Result<String, Error> {
    let metadata = inspect_totp(secret)?;
    let secret = extract_totp_secret(secret)?;
    let counter = timestamp / metadata.period;
    let counter_bytes = counter.to_be_bytes();
    let mut mac =
        <Hmac<Sha1> as Mac>::new_from_slice(&secret).map_err(|_| Error::InvalidTotpSecret)?;
    mac.update(&counter_bytes);
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    Ok(format!("{:06}", binary % 1_000_000))
}

pub fn inspect_totp(value: &str) -> Result<TotpMetadata, Error> {
    let value = value.trim();
    if value
        .to_ascii_lowercase()
        .starts_with("otpauth-migration://")
    {
        let secret = decode_migration_secret(value)?;
        return Ok(TotpMetadata {
            source: "Google Authenticator migration URI (single account)",
            algorithm: "SHA1",
            digits: 6,
            period: 30,
            secret_bytes: secret.len(),
        });
    }
    if !value.to_ascii_lowercase().starts_with("otpauth://") {
        let secret = decode_base32_secret(value)?;
        return Ok(TotpMetadata {
            source: "raw Base32 secret",
            algorithm: "SHA1",
            digits: 6,
            period: 30,
            secret_bytes: secret.len(),
        });
    }
    let uri = Url::parse(value).map_err(|_| Error::InvalidTotpSecret)?;
    if uri.scheme() != "otpauth" || uri.host_str() != Some("totp") {
        return Err(Error::InvalidTotpSecret);
    }
    let mut algorithm = "SHA1".to_string();
    let mut digits = 6;
    let mut period = 30;
    let mut secret = None;
    for (key, value) in uri.query_pairs() {
        match key.as_ref() {
            "secret" => secret = Some(value.into_owned()),
            "algorithm" => algorithm = value.into_owned(),
            "digits" => digits = value.parse().map_err(|_| Error::InvalidTotpDigits)?,
            "period" => period = value.parse().map_err(|_| Error::UnsupportedTotpPeriod)?,
            _ => {}
        }
    }
    if !algorithm.eq_ignore_ascii_case("SHA1") {
        return Err(Error::UnsupportedTotpAlgorithm);
    }
    if digits != 6 {
        return Err(Error::InvalidTotpDigits);
    }
    if period != 30 {
        return Err(Error::UnsupportedTotpPeriod);
    }
    let secret = decode_base32_secret(&secret.ok_or(Error::InvalidTotpSecret)?)?;
    Ok(TotpMetadata {
        source: "otpauth URI",
        algorithm: "SHA1",
        digits,
        period,
        secret_bytes: secret.len(),
    })
}

fn extract_totp_secret(value: &str) -> Result<Vec<u8>, Error> {
    let value = value.trim();
    if value
        .to_ascii_lowercase()
        .starts_with("otpauth-migration://")
    {
        return decode_migration_secret(value);
    }
    if !value.to_ascii_lowercase().starts_with("otpauth://") {
        return decode_base32_secret(value);
    }

    let uri = Url::parse(value).map_err(|_| Error::InvalidTotpSecret)?;
    if uri.scheme() != "otpauth" || uri.host_str() != Some("totp") {
        return Err(Error::InvalidTotpSecret);
    }
    let mut secret = None;
    let mut algorithm = None;
    let mut digits = None;
    let mut period = None;
    for (key, value) in uri.query_pairs() {
        match key.as_ref() {
            "secret" => secret = Some(value.into_owned()),
            "algorithm" => algorithm = Some(value.into_owned()),
            "digits" => digits = Some(value.into_owned()),
            "period" => period = Some(value.into_owned()),
            _ => {}
        }
    }
    if algorithm
        .as_deref()
        .is_some_and(|value| !value.eq_ignore_ascii_case("SHA1"))
    {
        return Err(Error::UnsupportedTotpAlgorithm);
    }
    if digits.as_deref().is_some_and(|value| value != "6") {
        return Err(Error::InvalidTotpDigits);
    }
    if period.as_deref().is_some_and(|value| value != "30") {
        return Err(Error::UnsupportedTotpPeriod);
    }
    decode_base32_secret(&secret.ok_or(Error::InvalidTotpSecret)?)
}

fn decode_base32_secret(value: &str) -> Result<Vec<u8>, Error> {
    let normalized = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_uppercase()
        .trim_end_matches('=')
        .to_string();
    if normalized.is_empty() {
        return Err(Error::InvalidTotpSecret);
    }
    decode_base32(Alphabet::Rfc4648 { padding: false }, &normalized)
        .filter(|secret| !secret.is_empty())
        .ok_or(Error::InvalidTotpSecret)
}

#[derive(Message)]
struct MigrationPayload {
    #[prost(message, repeated, tag = "1")]
    otp_parameters: Vec<OtpParameters>,
}

#[derive(Message)]
struct OtpParameters {
    #[prost(bytes, tag = "1")]
    secret: Vec<u8>,
    #[prost(enumeration = "MigrationAlgorithm", tag = "4")]
    algorithm: i32,
    #[prost(enumeration = "MigrationDigits", tag = "5")]
    digits: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
enum MigrationAlgorithm {
    Unspecified = 0,
    Sha1 = 1,
    Sha256 = 2,
    Sha512 = 3,
    Md5 = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
enum MigrationDigits {
    Unspecified = 0,
    Six = 1,
    Eight = 2,
}

fn decode_migration_secret(value: &str) -> Result<Vec<u8>, Error> {
    let uri = Url::parse(value).map_err(|_| Error::InvalidTotpSecret)?;
    let encoded = uri
        .query_pairs()
        .find(|(key, _)| key == "data")
        .map(|(_, value)| value.into_owned())
        .ok_or(Error::InvalidTotpSecret)?;
    let data = general_purpose::URL_SAFE_NO_PAD
        .decode(&encoded)
        .or_else(|_| general_purpose::STANDARD.decode(&encoded))
        .map_err(|_| Error::InvalidTotpSecret)?;
    let payload =
        MigrationPayload::decode(data.as_slice()).map_err(|_| Error::InvalidTotpSecret)?;
    let mut parameters = payload.otp_parameters.into_iter();
    let parameter = parameters.next().ok_or(Error::InvalidTotpSecret)?;
    if parameters.next().is_some() {
        return Err(Error::UnsupportedTotpMigration);
    }
    if parameter.algorithm != 0 && parameter.algorithm != MigrationAlgorithm::Sha1 as i32 {
        return Err(Error::UnsupportedTotpAlgorithm);
    }
    if parameter.digits != 0 && parameter.digits != MigrationDigits::Six as i32 {
        return Err(Error::InvalidTotpDigits);
    }
    if parameter.secret.is_empty() {
        return Err(Error::InvalidTotpSecret);
    }
    Ok(parameter.secret)
}

fn derive_key(
    password: &[u8],
    salt: &[u8],
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
) -> Result<Zeroizing<[u8; 32]>, Error> {
    let _password_lock = memory::Locked::new(password);
    let params = Params::new(memory_kib, iterations, parallelism, Some(32))
        .map_err(|_| Error::InvalidPasswordOrVault)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    let _key_lock = memory::Locked::new(&key[..]);
    argon2
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|_| Error::InvalidPasswordOrVault)?;
    Ok(key)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut temp = PathBuf::from(path);
    temp.set_extension(format!("silo-tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        set_private_permissions(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if path.exists() {
            let backup = path.with_extension("vault.bak");
            let _ = fs::copy(path, backup);
        }
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temp, path)?;
        set_private_permissions(path)?;
        Ok::<(), std::io::Error>(())
    })();
    let _ = fs::remove_file(&temp);
    result.map_err(Error::Io)
}

fn set_private_permissions(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_output_is_redacted() {
        let secret = SecretString::new("do-not-print");
        let output = format!("{secret:?}");
        assert!(!output.contains("do-not-print"));
        assert!(output.contains("REDACTED"));
    }

    #[test]
    fn vault_round_trip_works() {
        let path = std::env::temp_dir().join(format!("silo-test-{}.vault", Uuid::new_v4()));
        let mut vault = Vault::new();
        vault.add(new_entry(
            "Example".into(),
            "https://example.com".into(),
            "alice".into(),
            "alice@example.com".into(),
            "correct horse battery staple".into(),
            Some("JBSWY3DPEHPK3PXP".into()),
        ));
        save_vault(&path, &vault, "master password").unwrap();
        assert_eq!(load_vault(&path, "master password").unwrap(), vault);
        assert!(matches!(
            load_vault(&path, "wrong"),
            Err(Error::InvalidPasswordOrVault)
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn totp_matches_known_vector() {
        assert_eq!(generate_totp("JBSWY3DPEHPK3PXP", 59).unwrap(), "996554");
        assert_eq!(
            generate_totp(
                "otpauth://totp/GitHub:alice?secret=JBSWY3DPEHPK3PXP&issuer=GitHub",
                59
            )
            .unwrap(),
            "996554"
        );

        let secret = decode_base32_secret("JBSWY3DPEHPK3PXP").unwrap();
        let mut parameter = vec![0x0a, secret.len() as u8];
        parameter.extend_from_slice(&secret);
        parameter.extend_from_slice(&[0x20, 0x01, 0x28, 0x01]);
        let mut payload = vec![0x0a, parameter.len() as u8];
        payload.extend_from_slice(&parameter);
        let encoded = general_purpose::URL_SAFE_NO_PAD.encode(payload);
        let uri = format!("otpauth-migration://offline?data={encoded}");
        assert_eq!(generate_totp(&uri, 59).unwrap(), "996554");
        let metadata =
            inspect_totp("otpauth://totp/GitHub:alice?secret=JBSWY3DPEHPK3PXP&issuer=GitHub")
                .unwrap();
        assert_eq!(metadata.source, "otpauth URI");
        assert_eq!(metadata.algorithm, "SHA1");
        assert_eq!(metadata.digits, 6);
        assert_eq!(metadata.period, 30);
        assert_eq!(metadata.secret_bytes, 10);
    }

    #[test]
    fn save_creates_backup_and_rejects_tampering() {
        let path = std::env::temp_dir().join(format!("silo-backup-{}.vault", Uuid::new_v4()));
        let vault = Vault::new();
        save_vault(&path, &vault, "master password").unwrap();
        save_vault(&path, &vault, "master password").unwrap();
        assert!(path.with_extension("vault.bak").exists());
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            load_vault(&path, "master password"),
            Err(Error::InvalidPasswordOrVault)
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_vault_inputs_fail_without_panicking() {
        for length in 0..256usize {
            let path = std::env::temp_dir().join(format!("silo-malformed-{length}.vault"));
            let bytes = (0..length)
                .map(|index| ((index * 73 + 19) % 256) as u8)
                .collect::<Vec<_>>();
            fs::write(&path, bytes).unwrap();
            assert!(load_vault(&path, "password").is_err());
            let _ = fs::remove_file(path);
        }
    }
}
