use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    net::TcpStream,
    path::PathBuf,
};
use zeroize::Zeroize;

#[derive(Debug, Parser)]
#[command(
    name = "silo-native-host",
    about = "Local native-messaging bridge for the Silo browser extension"
)]
struct Args {
    #[arg(short, long)]
    _vault: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
enum Request {
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
}

#[derive(Debug, Serialize)]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    otp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<MatchItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unlocked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
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

#[derive(Debug, Deserialize, Serialize)]
struct MatchItem {
    id: String,
    name: String,
    username: String,
}

#[derive(Debug, Deserialize)]
struct BrokerResponse {
    ok: bool,
    username: Option<String>,
    password: Option<String>,
    otp: Option<String>,
    matches: Option<Vec<MatchItem>>,
    unlocked: Option<bool>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrokerState {
    address: String,
    token: String,
}

#[derive(Debug, Serialize)]
struct Envelope<'a> {
    token: &'a str,
    request: &'a Request,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = Args::parse();
    let mut input = io::stdin();
    let mut output = io::stdout();
    loop {
        let Some(message) = read_message(&mut input)? else {
            break;
        };
        let response = match serde_json::from_slice::<Request>(&message) {
            Ok(request) => request_broker(&request),
            Err(_) => error_response("invalid request"),
        };
        write_message(&mut output, &serde_json::to_vec(&response)?)?;
    }
    Ok(())
}

fn request_broker(request: &Request) -> Response {
    let state_path = broker_state_path();
    let state = match fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BrokerState>(&bytes).ok())
    {
        Some(state) => state,
        None => return error_response("Silo broker unavailable; run silo broker first"),
    };
    let mut stream = match TcpStream::connect(&state.address) {
        Ok(stream) => stream,
        Err(_) => return error_response("Silo broker is not running"),
    };
    let envelope = match serde_json::to_vec(&Envelope {
        token: &state.token,
        request,
    }) {
        Ok(envelope) => envelope,
        Err(_) => return error_response("could not encode broker request"),
    };
    if write_message(&mut stream, &envelope).is_err() {
        return error_response("could not contact Silo broker");
    }
    let Some(response) = read_message(&mut stream).ok().flatten() else {
        return error_response("Silo broker returned no response");
    };
    match serde_json::from_slice::<BrokerResponse>(&response) {
        Ok(mut response) => Response {
            ok: response.ok,
            username: response.username.take(),
            password: response.password.take(),
            otp: response.otp.take(),
            matches: response.matches.take(),
            unlocked: response.unlocked,
            error: response.error.take(),
        },
        Err(_) => error_response("invalid Silo broker response"),
    }
}

fn error_response(error: &str) -> Response {
    Response {
        ok: false,
        username: None,
        password: None,
        otp: None,
        matches: None,
        unlocked: None,
        error: Some(error.into()),
    }
}

fn broker_state_path() -> PathBuf {
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
