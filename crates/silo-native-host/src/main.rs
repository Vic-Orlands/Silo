use clap::Parser;
use silo_protocol::{
    broker_state_path, read_frame, write_frame, Envelope, Request, Response, PROTOCOL_VERSION,
    REQUEST_TTL_SECS,
};
use std::{io, net::TcpStream, path::PathBuf, process::Command, time::Duration};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "silo-native-host",
    about = "Local native-messaging bridge for the Silo browser extension"
)]
struct Args {
    #[arg(short, long)]
    _vault: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = Args::parse();
    let mut input = io::stdin();
    let mut output = io::stdout();
    loop {
        let Some(message) = read_frame(&mut input)? else {
            break;
        };
        let response = match serde_json::from_slice::<Request>(&message) {
            Ok(Request::OpenSilo) => open_silo(),
            Ok(request) => request_broker(&request),
            Err(_) => error_response("invalid request"),
        };
        write_frame(&mut output, &serde_json::to_vec(&response)?)?;
    }
    Ok(())
}

fn open_silo() -> Response {
    let binary = silo_binary();
    match Command::new(binary).arg("shell").spawn() {
        Ok(_) => Response {
            ok: true,
            request_id: None,
            username: None,
            password: None,
            otp: None,
            matches: None,
            unlocked: None,
            error: None,
        },
        Err(error) => error_response(&format!("could not open Silo: {error}")),
    }
}

fn silo_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("SILO_BIN") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let cargo_binary = PathBuf::from(home).join(".cargo/bin/silo");
        if cargo_binary.is_file() {
            return cargo_binary;
        }
    }
    PathBuf::from("silo")
}

fn request_broker(request: &Request) -> Response {
    let state_path = broker_state_path();
    let state = match silo_protocol::read_state(&state_path) {
        Ok(state) => state,
        Err(_) => return error_response("Silo broker unavailable; run silo broker first"),
    };
    let mut stream = match TcpStream::connect(&state.address) {
        Ok(stream) => stream,
        Err(_) => return error_response("Silo broker is not running"),
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(REQUEST_TTL_SECS)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(REQUEST_TTL_SECS)));
    let envelope = match serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        expires_at: now().saturating_add(REQUEST_TTL_SECS),
        token: state.token,
        request: request.clone(),
    }) {
        Ok(envelope) => envelope,
        Err(_) => return error_response("could not encode broker request"),
    };
    if write_frame(&mut stream, &envelope).is_err() {
        return error_response("could not contact Silo broker");
    }
    let Some(response) = read_frame(&mut stream).ok().flatten() else {
        return error_response("Silo broker returned no response");
    };
    match serde_json::from_slice::<Response>(&response) {
        Ok(response) => response,
        Err(_) => error_response("invalid Silo broker response"),
    }
}

fn error_response(error: &str) -> Response {
    Response {
        ok: false,
        request_id: None,
        username: None,
        password: None,
        otp: None,
        matches: None,
        unlocked: None,
        error: Some(error.into()),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_messages_round_trip_with_little_endian_length() {
        let message = br#"{"type":"get_otp","url":"https://github.com"}"#;
        let mut encoded = Vec::new();
        write_frame(&mut encoded, message).unwrap();
        assert_eq!(&encoded[..4], &(message.len() as u32).to_le_bytes());
        assert_eq!(
            read_frame(&mut encoded.as_slice()).unwrap(),
            Some(message.to_vec())
        );
    }

    #[test]
    fn native_messages_reject_oversized_frames() {
        let encoded = (1_000_001u32).to_le_bytes().to_vec();
        assert!(read_frame(&mut encoded.as_slice()).is_err());
    }
}
