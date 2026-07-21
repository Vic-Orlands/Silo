use argon2::{Algorithm, Argon2, Params, Version};
use base32::{decode as decode_base32, Alphabet};
use base64::{engine::general_purpose, Engine as _};
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use prost::Message;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha1::Sha1;
use std::{fs, path::Path};
use thiserror::Error;
use url::Url;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const MAGIC: &[u8; 8] = b"SILO\0\0\0\0";
const LEGACY_MAGIC: &[u8; 8] = b"UZOPASS\0";
const FORMAT_VERSION: u8 = 1;
const SALT_LENGTH: usize = 16;
const NONCE_LENGTH: usize = 24;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SecretString(pub String);

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
        let current_host = Url::parse(current_url).ok()?.host_str()?.to_lowercase();
        self.entries.iter().find(|entry| {
            let Some(entry_host) = Url::parse(&entry.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_lowercase))
            else {
                return false;
            };
            current_host == entry_host || current_host.ends_with(&format!(".{entry_host}"))
        })
    }
}

pub fn save_vault(path: impl AsRef<Path>, vault: &Vault, password: &str) -> Result<(), Error> {
    let plaintext = serde_json::to_vec(vault)?;
    let mut salt = [0u8; SALT_LENGTH];
    let mut nonce = [0u8; NONCE_LENGTH];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let key = derive_key(password.as_bytes(), &salt)?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| Error::InvalidPasswordOrVault)?;

    let mut bytes =
        Vec::with_capacity(MAGIC.len() + 1 + SALT_LENGTH + NONCE_LENGTH + ciphertext.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(FORMAT_VERSION);
    bytes.extend_from_slice(&salt);
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&ciphertext);

    fs::write(&path, bytes)?;
    set_private_permissions(path.as_ref())?;
    Ok(())
}

pub fn load_vault(path: impl AsRef<Path>, password: &str) -> Result<Vault, Error> {
    let bytes = fs::read(path)?;
    if bytes.len() < MAGIC.len() + 1 + SALT_LENGTH + NONCE_LENGTH
        || (&bytes[..MAGIC.len()] != MAGIC && &bytes[..MAGIC.len()] != LEGACY_MAGIC)
    {
        return Err(Error::UnsupportedFormat);
    }
    if bytes[MAGIC.len()] != FORMAT_VERSION {
        return Err(Error::UnsupportedFormat);
    }

    let salt_start = MAGIC.len() + 1;
    let nonce_start = salt_start + SALT_LENGTH;
    let ciphertext_start = nonce_start + NONCE_LENGTH;
    let salt = &bytes[salt_start..nonce_start];
    let nonce = &bytes[nonce_start..ciphertext_start];
    let ciphertext = &bytes[ciphertext_start..];

    let key = derive_key(password.as_bytes(), salt)?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let plaintext = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| Error::InvalidPasswordOrVault)?;

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

pub fn generate_totp(secret: &str, timestamp: u64) -> Result<String, Error> {
    let secret = extract_totp_secret(secret)?;
    let counter = timestamp / 30;
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
    decode_base32(Alphabet::Rfc4648 { padding: false }, &normalized).ok_or(Error::InvalidTotpSecret)
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
    let parameter = payload
        .otp_parameters
        .into_iter()
        .next()
        .ok_or(Error::InvalidTotpSecret)?;
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

fn derive_key(password: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, Error> {
    let params =
        Params::new(64 * 1024, 3, 1, Some(32)).map_err(|_| Error::InvalidPasswordOrVault)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|_| Error::InvalidPasswordOrVault)?;
    Ok(key)
}

fn set_private_permissions(path: &Path) -> Result<(), std::io::Error> {
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
    }
}
