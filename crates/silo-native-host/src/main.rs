use clap::Parser;
use silo_protocol::{
    broker_state_path, read_frame, write_frame, Envelope, Request, Response, PROTOCOL_VERSION,
    REQUEST_TTL_SECS,
};
use std::{
    io,
    net::TcpStream,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
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
    while let Some(message) = read_frame(&mut input)? {
        let response = match serde_json::from_slice::<Request>(&message) {
            Ok(Request::OpenSilo) => open_silo(),
            Ok(request) => request_broker(request),
            Err(_) => error_response("invalid request"),
        };
        write_frame(&mut output, &serde_json::to_vec(&response)?)?;
    }
    Ok(())
}

fn open_silo() -> Response {
    let binary = silo_binary();
    let vault = silo_protocol::read_state(broker_state_path())
        .ok()
        .map(|state| state.vault_path);
    let vault = vault.filter(|path| !path.as_os_str().is_empty());
    let vault = vault.or_else(|| std::env::var_os("SILO_VAULT").map(PathBuf::from));
    if let Err(error) = ensure_broker(&binary, vault.as_ref()) {
        return error_response(&error);
    }

    #[cfg(target_os = "macos")]
    {
        unlock_with_native_dialog()
    }

    #[cfg(not(target_os = "macos"))]
    {
        match launch_silo(binary, vault, "unlock") {
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
}

#[cfg(target_os = "macos")]
fn unlock_with_native_dialog() -> Response {
    let script = r#"
set resultRecord to display dialog "Unlock Silo" with title "Silo" default answer "" buttons {"Cancel", "Unlock"} default button "Unlock" cancel button "Cancel" with hidden answer
return text returned of resultRecord
"#;
    let output = match Command::new("osascript").args(["-e", script]).output() {
        Ok(output) => output,
        Err(error) => {
            return error_response(&format!("could not open Silo unlock dialog: {error}"))
        }
    };
    if !output.status.success() {
        return error_response("Silo unlock cancelled");
    }
    let password = String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if password.is_empty() {
        return error_response("Silo password cannot be empty");
    }
    request_broker(Request::Unlock {
        password: silo_protocol::SensitiveString::new(password),
    })
}

fn ensure_broker(binary: &PathBuf, vault: Option<&PathBuf>) -> Result<(), String> {
    let state_path = broker_state_path();
    if let Ok(state) = silo_protocol::read_state(&state_path) {
        if TcpStream::connect(&state.address).is_ok() {
            return Ok(());
        }
    }

    let vault = vault
        .cloned()
        .unwrap_or_else(|| PathBuf::from("silo.vault"));
    let mut command = Command::new(binary);
    command
        .args([
            "--vault",
            &vault.to_string_lossy(),
            "broker",
            "--background",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .spawn()
        .map_err(|error| format!("could not start Silo broker: {error}"))?;

    for _ in 0..40 {
        if let Ok(state) = silo_protocol::read_state(&state_path) {
            if TcpStream::connect(&state.address).is_ok() {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("Silo broker did not start".into())
}

#[cfg(not(target_os = "macos"))]
fn launch_silo(
    binary: PathBuf,
    vault: Option<PathBuf>,
    subcommand: &str,
) -> io::Result<std::process::Child> {
    let mut command = Command::new(binary);
    if let Some(vault) = vault {
        command.arg("--vault").arg(vault);
    }
    command.arg(subcommand).spawn()
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

fn request_broker(request: Request) -> Response {
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
        request,
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
