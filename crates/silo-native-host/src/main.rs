use clap::Parser;
use serde::{Deserialize, Serialize};
use silo_core::load_vault;
use std::{
    io::{self, Read, Write},
    path::PathBuf,
};

#[derive(Debug, Parser)]
#[command(
    name = "silo-native-host",
    about = "Local native-messaging bridge for the Silo browser extension"
)]
struct Args {
    #[arg(short, long)]
    vault: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Request {
    #[serde(rename = "get_login")]
    GetLogin { url: String },
    #[serde(rename = "get_otp")]
    GetOtp { url: String },
}

#[derive(Debug, Serialize)]
struct Response<'a> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    otp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let vault_path = args.vault.unwrap_or_else(default_vault_path);
    eprintln!("Silo native host: unlock the vault to enable browser autofill.");
    let password = rpassword::prompt_password("Vault password: ")?;
    let vault = load_vault(&vault_path, &password)?;
    let mut input = io::stdin();
    let mut output = io::stdout();

    loop {
        let Some(message) = read_message(&mut input)? else {
            break;
        };
        let request: Request = match serde_json::from_slice(&message) {
            Ok(request) => request,
            Err(_) => {
                write_message(
                    &mut output,
                    &serde_json::to_vec(&Response {
                        ok: false,
                        username: None,
                        password: None,
                        otp: None,
                        error: Some("invalid request"),
                    })?,
                )?;
                continue;
            }
        };

        let response = match request {
            Request::GetLogin { url } => match vault.find_for_url(&url) {
                Some(entry) => Response {
                    ok: true,
                    username: Some(&entry.username),
                    password: Some(entry.password.as_str()),
                    otp: None,
                    error: None,
                },
                None => Response {
                    ok: false,
                    username: None,
                    password: None,
                    otp: None,
                    error: Some("no matching login"),
                },
            },
            Request::GetOtp { url } => match vault.find_for_url(&url) {
                Some(entry) => match entry
                    .totp_secret
                    .as_ref()
                    .and_then(|secret| silo_core::generate_totp(secret.as_str(), now()).ok())
                {
                    Some(otp) => Response {
                        ok: true,
                        username: None,
                        password: None,
                        otp: Some(otp),
                        error: None,
                    },
                    None => Response {
                        ok: false,
                        username: None,
                        password: None,
                        otp: None,
                        error: Some("no TOTP secret"),
                    },
                },
                None => Response {
                    ok: false,
                    username: None,
                    password: None,
                    otp: None,
                    error: Some("no matching login"),
                },
            },
        };
        write_message(&mut output, &serde_json::to_vec(&response)?)?;
    }
    Ok(())
}

fn default_vault_path() -> PathBuf {
    if let Ok(path) = std::env::var("SILO_VAULT_PATH") {
        return PathBuf::from(path);
    }
    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join("Library/Application Support/Silo/silo.vault");
    }
    #[cfg(target_os = "windows")]
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("Silo/silo.vault");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/silo/silo.vault");
    }
    PathBuf::from("silo.vault")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_message(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0u8; 4];
    match input.read_exact(&mut length) {
        Ok(()) => {
            let length = u32::from_le_bytes(length) as usize;
            if length > 1_000_000 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "message too large",
                ));
            }
            let mut message = vec![0u8; length];
            input.read_exact(&mut message)?;
            Ok(Some(message))
        }
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_message(output: &mut impl Write, message: &[u8]) -> io::Result<()> {
    output.write_all(&(message.len() as u32).to_le_bytes())?;
    output.write_all(message)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_messages_round_trip_with_little_endian_length() {
        let message = br#"{"type":"get_otp","url":"https://github.com"}"#;
        let mut encoded = Vec::new();
        write_message(&mut encoded, message).unwrap();
        assert_eq!(&encoded[..4], &(message.len() as u32).to_le_bytes());
        assert_eq!(
            read_message(&mut encoded.as_slice()).unwrap(),
            Some(message.to_vec())
        );
    }

    #[test]
    fn native_messages_reject_oversized_frames() {
        let encoded = (1_000_001u32).to_le_bytes().to_vec();
        assert!(read_message(&mut encoded.as_slice()).is_err());
    }
}
