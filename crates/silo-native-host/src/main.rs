use clap::Parser;
use silo_protocol::{
    broker_state_path, read_frame, write_frame, Envelope, Request, Response, PROTOCOL_VERSION,
};
use std::{io, net::TcpStream, path::PathBuf};

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
            Ok(request) => request_broker(&request),
            Err(_) => error_response("invalid request"),
        };
        write_frame(&mut output, &serde_json::to_vec(&response)?)?;
    }
    Ok(())
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
    let envelope = match serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
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
        username: None,
        password: None,
        otp: None,
        matches: None,
        unlocked: None,
        error: Some(error.into()),
    }
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
