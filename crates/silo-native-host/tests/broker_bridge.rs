use silo_broker::start_with_vault;
use silo_core::{load_vault, new_entry, save_vault, Vault};
use silo_protocol::{
    read_frame, write_frame, BrokerState, Envelope, Request, Response, PROTOCOL_VERSION,
    REQUEST_TTL_SECS,
};
use std::{
    fs,
    process::{Command, Stdio},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn native_host_round_trips_login_and_totp_requests() {
    let _guard = TEST_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("silo-bridge-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let state_path = root.join("broker.json");
    let vault_path = root.join("test.vault");
    std::env::set_var("SILO_BROKER_STATE_PATH", &state_path);

    let mut vault = Vault::new();
    vault.add(new_entry(
        "GitHub".into(),
        "https://github.com".into(),
        "alice@example.com".into(),
        String::new(),
        "correct horse".into(),
        Some("JBSWY3DPEHPK3PXP".into()),
    ));
    save_vault(&vault_path, &vault, "master password").unwrap();
    let broker = start_with_vault(
        vault,
        vault_path.clone(),
        Zeroizing::new(String::from("master password")),
        900,
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_silo-native-host"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    write_frame(
        &mut input,
        &serde_json::to_vec(&Request::GetLogin {
            url: "https://github.com/login".into(),
            entry_id: None,
        })
        .unwrap(),
    )
    .unwrap();
    let response: Response =
        serde_json::from_slice(&read_frame(&mut output).unwrap().unwrap()).unwrap();
    assert!(response.ok);
    assert!(response.request_id.as_deref().is_some());
    assert_eq!(response.username.as_deref(), Some("alice@example.com"));
    assert_eq!(response.password.as_deref(), Some("correct horse"));

    write_frame(
        &mut input,
        &serde_json::to_vec(&Request::GetOtp {
            url: "https://github.com".into(),
        })
        .unwrap(),
    )
    .unwrap();
    let response: Response =
        serde_json::from_slice(&read_frame(&mut output).unwrap().unwrap()).unwrap();
    assert!(response.ok);
    assert!(response.request_id.as_deref().is_some());
    assert_eq!(response.otp.as_ref().map(String::len), Some(6));

    write_frame(
        &mut input,
        &serde_json::to_vec(&Request::GetLogin {
            url: "https://not-github.example".into(),
            entry_id: None,
        })
        .unwrap(),
    )
    .unwrap();
    let response: Response =
        serde_json::from_slice(&read_frame(&mut output).unwrap().unwrap()).unwrap();
    assert!(!response.ok);
    assert!(response.error.as_deref().unwrap().contains("matching"));

    write_frame(
        &mut input,
        &serde_json::to_vec(&Request::SaveLogin {
            url: "https://example.com/login".into(),
            username: "new@example.com".into(),
            password: "new password".into(),
        })
        .unwrap(),
    )
    .unwrap();
    let response: Response =
        serde_json::from_slice(&read_frame(&mut output).unwrap().unwrap()).unwrap();
    assert!(response.ok);
    assert!(load_vault(&vault_path, "master password")
        .unwrap()
        .find("new@example.com")
        .is_some());

    broker.lock();
    write_frame(
        &mut input,
        &serde_json::to_vec(&Request::GetLogin {
            url: "https://github.com".into(),
            entry_id: None,
        })
        .unwrap(),
    )
    .unwrap();
    let response: Response =
        serde_json::from_slice(&read_frame(&mut output).unwrap().unwrap()).unwrap();
    assert!(!response.ok);
    assert!(response.request_id.as_deref().is_some());
    assert!(response.error.as_deref().unwrap().contains("locked"));

    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    drop(broker);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_host_rejects_expired_requests() {
    let _guard = TEST_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("silo-expired-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let state_path = root.join("broker.json");
    let vault_path = root.join("test.vault");
    std::env::set_var("SILO_BROKER_STATE_PATH", &state_path);
    save_vault(&vault_path, &Vault::new(), "master password").unwrap();
    let broker = start_with_vault(
        Vault::new(),
        vault_path,
        Zeroizing::new(String::from("master password")),
        900,
    )
    .unwrap();
    let state: BrokerState = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let mut stream = std::net::TcpStream::connect(state.address).unwrap();
    let envelope = Envelope {
        version: PROTOCOL_VERSION,
        request_id: "expired-1".into(),
        expires_at: now().saturating_sub(1),
        token: state.token,
        request: Request::Status,
    };
    write_frame(&mut stream, &serde_json::to_vec(&envelope).unwrap()).unwrap();
    let response: Response =
        serde_json::from_slice(&read_frame(&mut stream).unwrap().unwrap()).unwrap();
    assert!(!response.ok);
    assert_eq!(response.request_id.as_deref(), Some("expired-1"));
    assert_eq!(response.error.as_deref(), Some("broker request expired"));

    let envelope = Envelope {
        version: PROTOCOL_VERSION,
        request_id: "token-1".into(),
        expires_at: now() + REQUEST_TTL_SECS,
        token: "wrong-token".into(),
        request: Request::Status,
    };
    let mut stream =
        std::net::TcpStream::connect(silo_protocol::read_state(&state_path).unwrap().address)
            .unwrap();
    write_frame(&mut stream, &serde_json::to_vec(&envelope).unwrap()).unwrap();
    let response: Response =
        serde_json::from_slice(&read_frame(&mut stream).unwrap().unwrap()).unwrap();
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("invalid broker token"));
    drop(broker);
    let _ = fs::remove_dir_all(root);
}
