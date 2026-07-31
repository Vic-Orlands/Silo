use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
};
use zeroize::Zeroize;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_FRAME_SIZE: usize = 1_000_000;
pub const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 900;
pub const REQUEST_TTL_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerState {
    pub address: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u8,
    pub request_id: String,
    pub expires_at: u64,
    pub token: String,
    pub request: Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    #[serde(rename = "status")]
    Status,
    #[serde(rename = "get_matches")]
    GetMatches { url: String },
    #[serde(rename = "get_login")]
    GetLogin {
        url: String,
        #[serde(default)]
        entry_id: Option<String>,
    },
    #[serde(rename = "get_otp")]
    GetOtp { url: String },
    #[serde(rename = "save_login")]
    SaveLogin {
        url: String,
        username: String,
        password: String,
    },
    #[serde(rename = "open_silo")]
    OpenSilo,
    #[serde(rename = "lock")]
    Lock,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<MatchItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlocked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Drop for Response {
    fn drop(&mut self) {
        if let Some(mut value) = self.username.take() {
            value.zeroize();
        }
        if let Some(mut value) = self.password.take() {
            value.zeroize();
        }
        if let Some(mut value) = self.otp.take() {
            value.zeroize();
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MatchItem {
    pub id: String,
    pub name: String,
    pub username: String,
}

pub fn read_frame(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0u8; 4];
    match input.read_exact(&mut length) {
        Ok(()) => {
            let length = u32::from_le_bytes(length) as usize;
            if length > MAX_FRAME_SIZE {
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

pub fn write_frame(output: &mut impl Write, message: &[u8]) -> io::Result<()> {
    if message.len() > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message too large",
        ));
    }
    output.write_all(&(message.len() as u32).to_le_bytes())?;
    output.write_all(message)?;
    output.flush()
}

pub fn broker_state_path() -> PathBuf {
    if let Ok(path) = std::env::var("SILO_BROKER_STATE_PATH") {
        return PathBuf::from(path);
    }
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime).join("silo-broker.json");
    }
    if let Ok(home) = std::env::var("HOME") {
        #[cfg(target_os = "macos")]
        return PathBuf::from(home).join("Library/Application Support/Silo/broker.json");
        #[cfg(not(target_os = "macos"))]
        return PathBuf::from(home).join(".local/state/silo/broker.json");
    }
    PathBuf::from("silo-broker.json")
}

pub fn read_state(path: impl AsRef<std::path::Path>) -> io::Result<BrokerState> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips_with_version() {
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            request_id: "request-1".into(),
            expires_at: 1,
            token: "token".into(),
            request: Request::GetOtp {
                url: "https://example.com".into(),
            },
        };
        let encoded = serde_json::to_vec(&envelope).unwrap();
        let decoded: Envelope = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(decoded.request_id, "request-1");
    }

    #[test]
    fn frames_reject_oversized_messages() {
        let encoded = (MAX_FRAME_SIZE as u32 + 1).to_le_bytes().to_vec();
        assert!(read_frame(&mut encoded.as_slice()).is_err());
        assert!(write_frame(&mut Vec::new(), &vec![0; MAX_FRAME_SIZE + 1]).is_err());
    }
}
