use clap::Parser;
use rand::{thread_rng, RngCore};
use rpassword::prompt_password;
use silo_core::{generate_totp, load_vault, new_entry, save_vault, Vault};
use silo_protocol::{
    broker_state_path, read_frame, write_frame, BrokerState, Envelope, MatchItem, Request,
    Response, PROTOCOL_VERSION,
};
use std::{
    fs, io,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Debug, Parser)]
#[command(name = "silo-broker", about = "Local unlocked Silo session broker")]
struct Args {
    #[arg(short, long, default_value = "silo.vault")]
    vault: PathBuf,
    #[arg(long, default_value_t = silo_protocol::DEFAULT_SESSION_TIMEOUT_SECS)]
    timeout: u64,
    #[arg(long)]
    background: bool,
}

struct Session {
    vault: Option<Vault>,
    vault_path: PathBuf,
    master: Option<Zeroizing<String>>,
    last_activity: Instant,
    timeout: Duration,
}

pub struct BrokerHandle {
    session: Arc<Mutex<Session>>,
    state_path: PathBuf,
}

impl BrokerHandle {
    pub fn lock(&self) {
        if let Ok(mut session) = self.session.lock() {
            session.vault = None;
            session.master = None;
            session.last_activity = Instant::now();
        }
    }
}

impl Drop for BrokerHandle {
    fn drop(&mut self) {
        self.lock();
        let _ = fs::remove_file(&self.state_path);
    }
}

#[allow(dead_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.background {
        return run_background(args.vault, args.timeout);
    }
    run_with_config(args.vault, args.timeout)
}

pub fn run_background(vault_path: PathBuf, timeout: u64) -> Result<(), Box<dyn std::error::Error>> {
    let _broker = start_locked(vault_path, timeout)?;
    println!("Silo broker is running in the background and locked.");
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
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
    let broker = start_with_vault(vault, vault_path, password, timeout)?;

    println!("Silo broker is unlocked.");
    println!("Browser requests can now be approved through the local session.");
    println!("Type 'lock' to lock or 'q' to quit.");
    let mut command = String::new();
    while io::stdin().read_line(&mut command)? > 0 {
        match command.trim() {
            "lock" => {
                broker.lock();
                println!("Silo broker locked.");
            }
            "q" | "quit" | "exit" => break,
            _ => println!("Commands: lock, q"),
        }
        command.clear();
    }
    Ok(())
}

pub fn start_with_vault(
    vault: Vault,
    vault_path: PathBuf,
    master: Zeroizing<String>,
    timeout: u64,
) -> Result<BrokerHandle, Box<dyn std::error::Error>> {
    if timeout == 0 {
        return Err("broker timeout must be greater than zero".into());
    }

    start_session(Some(vault), vault_path, Some(master), timeout)
}

pub fn start_locked(
    vault_path: PathBuf,
    timeout: u64,
) -> Result<BrokerHandle, Box<dyn std::error::Error>> {
    if !vault_path.is_file() {
        return Err(format!("vault not found: {}", vault_path.display()).into());
    }
    start_session(None, vault_path, None, timeout)
}

fn start_session(
    vault: Option<Vault>,
    vault_path: PathBuf,
    master: Option<Zeroizing<String>>,
    timeout: u64,
) -> Result<BrokerHandle, Box<dyn std::error::Error>> {
    if timeout == 0 {
        return Err("broker timeout must be greater than zero".into());
    }

    let vault_path = fs::canonicalize(&vault_path).unwrap_or(vault_path);

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
            vault_path: vault_path.clone(),
        },
    )?;

    let session = Arc::new(Mutex::new(Session {
        vault,
        vault_path,
        master,
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

    Ok(BrokerHandle {
        session,
        state_path,
    })
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
    let mut response = if envelope.version != PROTOCOL_VERSION {
        error_response("unsupported broker protocol version")
    } else if envelope.expires_at < now() {
        error_response("broker request expired")
    } else if envelope.token != expected_token {
        Response {
            ok: false,
            request_id: Some(envelope.request_id.clone()),
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
    response.request_id = Some(envelope.request_id);
    write_frame(&mut stream, &serde_json::to_vec(&response)?)?;
    Ok(())
}

fn handle_request(request: Request, session: &Arc<Mutex<Session>>) -> Response {
    let Ok(mut session) = session.lock() else {
        return error_response("broker session unavailable");
    };
    if session.last_activity.elapsed() >= session.timeout {
        session.vault = None;
        session.master = None;
    }
    match request {
        Request::Status => Response {
            ok: true,
            request_id: None,
            username: None,
            password: None,
            otp: None,
            matches: None,
            unlocked: Some(session.vault.is_some()),
            error: None,
        },
        Request::Lock => {
            session.vault = None;
            session.master = None;
            session.last_activity = Instant::now();
            Response {
                ok: true,
                request_id: None,
                username: None,
                password: None,
                otp: None,
                matches: None,
                unlocked: Some(false),
                error: None,
            }
        }
        Request::Unlock { password } => {
            if session.vault.is_some() {
                return Response {
                    ok: true,
                    request_id: None,
                    username: None,
                    password: None,
                    otp: None,
                    matches: None,
                    unlocked: Some(true),
                    error: None,
                };
            }
            match load_vault(&session.vault_path, password.as_str()) {
                Ok(vault) => {
                    session.vault = Some(vault);
                    session.master = Some(Zeroizing::new(password.as_str().to_owned()));
                    session.last_activity = Instant::now();
                    Response {
                        ok: true,
                        request_id: None,
                        username: None,
                        password: None,
                        otp: None,
                        matches: None,
                        unlocked: Some(true),
                        error: None,
                    }
                }
                Err(_) => error_response("could not unlock vault: invalid password"),
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
                    email: entry.email.clone(),
                })
                .collect();
            Response {
                ok: true,
                request_id: None,
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
                    request_id: None,
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
                        request_id: None,
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
        Request::SaveLogin {
            url,
            username,
            password,
        } => {
            if username.trim().is_empty() || password.is_empty() {
                return error_response("username and password are required");
            }
            let Ok(parsed) = url::Url::parse(&url) else {
                return error_response("login URL is invalid");
            };
            if !matches!(parsed.scheme(), "http" | "https") {
                return error_response("login URL must use HTTP or HTTPS");
            }
            let Some(host) = parsed.host_str() else {
                return error_response("login URL has no host");
            };
            let Some(mut vault) = session.vault.take() else {
                return locked_response();
            };
            let Some(master) = session.master.as_ref() else {
                session.vault = Some(vault);
                return locked_response();
            };
            if vault.entries.iter().any(|entry| {
                entry.username == username
                    && entry.password.as_str() == password
                    && url::Url::parse(&entry.url)
                        .ok()
                        .and_then(|entry_url| entry_url.host_str().map(str::to_owned))
                        .is_some_and(|entry_host| entry_host.eq_ignore_ascii_case(host))
            }) {
                session.vault = Some(vault);
                return error_response("this login is already saved");
            }
            vault.add(new_entry(
                host.to_string(),
                url,
                username,
                String::new(),
                password,
                None,
            ));
            if let Err(error) = save_vault(&session.vault_path, &vault, master) {
                session.vault = Some(vault);
                return error_response(&format!("could not save login: {error}"));
            }
            session.vault = Some(vault);
            Response {
                ok: true,
                request_id: None,
                username: None,
                password: None,
                otp: None,
                matches: None,
                unlocked: Some(true),
                error: None,
            }
        }
        Request::OpenSilo => error_response("Silo is already running in this session"),
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

fn locked_response() -> Response {
    error_response("vault is locked; unlock Silo first")
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
            vault_path: PathBuf::from("test.vault"),
            master: Some(Zeroizing::new(String::from("test"))),
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
            vault_path: PathBuf::from("test.vault"),
            master: Some(Zeroizing::new(String::from("test"))),
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
            vault_path: PathBuf::from("test.vault"),
            master: Some(Zeroizing::new(String::from("test"))),
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

    #[test]
    fn locked_session_unlocks_against_its_recorded_vault() {
        let path = std::env::temp_dir().join(format!(
            "silo-broker-unlock-{}-{}.vault",
            std::process::id(),
            now()
        ));
        save_vault(&path, &Vault::new(), "correct password").unwrap();
        let session = Arc::new(Mutex::new(Session {
            vault: None,
            vault_path: path.clone(),
            master: None,
            last_activity: Instant::now(),
            timeout: Duration::from_secs(900),
        }));

        let wrong = handle_request(
            Request::Unlock {
                password: silo_protocol::SensitiveString::new("wrong password"),
            },
            &session,
        );
        assert!(!wrong.ok);

        let unlocked = handle_request(
            Request::Unlock {
                password: silo_protocol::SensitiveString::new("correct password"),
            },
            &session,
        );
        assert!(unlocked.ok);
        assert_eq!(unlocked.unlocked, Some(true));
        fs::remove_file(path).unwrap();
    }
}
