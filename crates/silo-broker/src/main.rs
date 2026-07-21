use clap::Parser;
use rand::{thread_rng, RngCore};
use rpassword::prompt_password;
use serde::{Deserialize, Serialize};
use silo_core::{generate_totp, load_vault, Vault};
use std::{
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroize;
use zeroize::Zeroizing;

#[derive(Debug, Parser)]
#[command(name = "silo-broker", about = "Local unlocked Silo session broker")]
struct Args {
    #[arg(short, long, default_value = "silo.vault")]
    vault: PathBuf,
    #[arg(long, default_value_t = 900)]
    timeout: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct BrokerState {
    address: String,
    token: String,
}

#[derive(Debug, Serialize, Deserialize)]
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
    #[serde(rename = "lock")]
    Lock,
}

#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    token: String,
    request: Request,
}

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
struct MatchItem {
    id: String,
    name: String,
    username: String,
}

struct Session {
    vault: Option<Vault>,
    last_activity: Instant,
    timeout: Duration,
}

#[allow(dead_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    run_with_config(args.vault, args.timeout)
}

pub fn run_with_config(
    vault_path: PathBuf,
    timeout: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if timeout == 0 {
        return Err("broker timeout must be greater than zero".into());
    }

    let password = Zeroizing::new(prompt_password("Silo password: ")?);
    let vault = load_vault(&vault_path, &password)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(false)?;

    let mut token_bytes = [0u8; 32];
    thread_rng().fill_bytes(&mut token_bytes);
    let token = token_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let state_path = broker_state_path();
    write_state(
        &state_path,
        &BrokerState {
            address: listener.local_addr()?.to_string(),
            token: token.clone(),
        },
    )?;

    let session = Arc::new(Mutex::new(Session {
        vault: Some(vault),
        last_activity: Instant::now(),
        timeout: Duration::from_secs(timeout),
    }));
    let worker_session = Arc::clone(&session);
    let worker_token = token.clone();
    let _worker = thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let session = Arc::clone(&worker_session);
                    let token = worker_token.clone();
                    thread::spawn(move || {
                        let _ = serve_connection(stream, &token, session);
                    });
                }
                Err(_) => break,
            }
        }
    });

    println!("Silo broker is unlocked.");
    println!("Browser requests can now be approved through the local session.");
    println!("Type 'lock' to lock or 'q' to quit.");
    let mut command = String::new();
    while io::stdin().read_line(&mut command)? > 0 {
        match command.trim() {
            "lock" => {
                if let Ok(mut session) = session.lock() {
                    session.vault = None;
                    session.last_activity = Instant::now();
                }
                println!("Silo broker locked.");
            }
            "q" | "quit" | "exit" => break,
            _ => println!("Commands: lock, q"),
        }
        command.clear();
    }
    let _ = fs::remove_file(state_path);
    Ok(())
}

fn serve_connection(
    mut stream: TcpStream,
    expected_token: &str,
    session: Arc<Mutex<Session>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(bytes) = read_frame(&mut stream)? else {
        return Ok(());
    };
    let envelope: Envelope = serde_json::from_slice(&bytes)?;
    let response = if envelope.token != expected_token {
        Response {
            ok: false,
            username: None,
            password: None,
            otp: None,
            matches: None,
            unlocked: None,
            error: Some("invalid broker token".into()),
        }
    } else {
        handle_request(envelope.request, &session)
    };
    write_frame(&mut stream, &serde_json::to_vec(&response)?)?;
    Ok(())
}

fn handle_request(request: Request, session: &Arc<Mutex<Session>>) -> Response {
    let Ok(mut session) = session.lock() else {
        return error_response("broker session unavailable");
    };
    if session.last_activity.elapsed() >= session.timeout {
        session.vault = None;
    }
    match request {
        Request::Status => Response {
            ok: true,
            username: None,
            password: None,
            otp: None,
            matches: None,
            unlocked: Some(session.vault.is_some()),
            error: None,
        },
        Request::Lock => {
            session.vault = None;
            session.last_activity = Instant::now();
            Response {
                ok: true,
                username: None,
                password: None,
                otp: None,
                matches: None,
                unlocked: Some(false),
                error: None,
            }
        }
        Request::GetMatches { url } => {
            session.last_activity = Instant::now();
            let Some(vault) = session.vault.as_ref() else {
                return locked_response();
            };
            let matches = vault
                .find_all_for_url(&url)
                .into_iter()
                .map(|entry| MatchItem {
                    id: entry.id.to_string(),
                    name: entry.name.clone(),
                    username: entry.username.clone(),
                })
                .collect();
            Response {
                ok: true,
                username: None,
                password: None,
                otp: None,
                matches: Some(matches),
                unlocked: Some(true),
                error: None,
            }
        }
        Request::GetLogin { url, entry_id } => {
            session.last_activity = Instant::now();
            let Some(vault) = session.vault.as_ref() else {
                return locked_response();
            };
            let entry = match entry_id {
                Some(id) => vault
                    .find_all_for_url(&url)
                    .into_iter()
                    .find(|entry| entry.id.to_string() == id),
                None => vault.find_for_url(&url),
            };
            match entry {
                Some(entry) => Response {
                    ok: true,
                    username: Some(entry.username.clone()),
                    password: Some(entry.password.as_str().to_string()),
                    otp: None,
                    matches: None,
                    unlocked: Some(true),
                    error: None,
                },
                None => error_response("no matching login"),
            }
        }
        Request::GetOtp { url } => {
            session.last_activity = Instant::now();
            let Some(vault) = session.vault.as_ref() else {
                return locked_response();
            };
            match vault.find_for_url(&url) {
                Some(entry) => match entry
                    .totp_secret
                    .as_ref()
                    .and_then(|secret| generate_totp(secret.as_str(), now()).ok())
                {
                    Some(otp) => Response {
                        ok: true,
                        username: None,
                        password: None,
                        otp: Some(otp),
                        matches: None,
                        unlocked: Some(true),
                        error: None,
                    },
                    None => error_response("no TOTP secret"),
                },
                None => error_response("no matching login"),
            }
        }
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

fn locked_response() -> Response {
    error_response("vault is locked; unlock Silo first")
}

fn read_frame(stream: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0u8; 4];
    match stream.read_exact(&mut length) {
        Ok(()) => {
            let length = u32::from_le_bytes(length) as usize;
            if length > 1_000_000 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "message too large",
                ));
            }
            let mut bytes = vec![0u8; length];
            stream.read_exact(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_frame(stream: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()
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

fn write_state(path: &Path, state: &BrokerState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(state).map_err(io::Error::other)?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes)?;
    set_private_permissions(&temp)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temp, path)?;
    set_private_permissions(path)
}

fn set_private_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
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
    use silo_core::new_entry;

    #[test]
    fn locked_session_does_not_release_credentials() {
        let session = Arc::new(Mutex::new(Session {
            vault: None,
            last_activity: Instant::now(),
            timeout: Duration::from_secs(900),
        }));
        let response = handle_request(
            Request::GetLogin {
                url: "https://example.com".into(),
                entry_id: None,
            },
            &session,
        );
        assert!(!response.ok);
        assert_eq!(
            response.error.as_deref(),
            Some("vault is locked; unlock Silo first")
        );
    }

    #[test]
    fn expired_session_locks_before_handling_requests() {
        let session = Arc::new(Mutex::new(Session {
            vault: Some(Vault::new()),
            last_activity: Instant::now() - Duration::from_secs(10),
            timeout: Duration::from_secs(1),
        }));
        let response = handle_request(Request::Status, &session);
        assert_eq!(response.unlocked, Some(false));
    }

    #[test]
    fn match_listing_contains_only_the_current_origin() {
        let mut vault = Vault::new();
        vault.add(new_entry(
            "GitHub".into(),
            "https://github.com".into(),
            "alice".into(),
            "alice@example.com".into(),
            "secret".into(),
            None,
        ));
        let session = Arc::new(Mutex::new(Session {
            vault: Some(vault),
            last_activity: Instant::now(),
            timeout: Duration::from_secs(900),
        }));
        let response = handle_request(
            Request::GetMatches {
                url: "https://github.com/login".into(),
            },
            &session,
        );
        assert_eq!(response.matches.as_ref().map(Vec::len), Some(1));
    }
}
