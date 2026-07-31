use clap::Parser;
use silo_broker::BrokerHandle;
use silo_protocol::{
    broker_state_path, read_frame, read_state, write_frame, Envelope, Request, Response,
    PROTOCOL_VERSION, REQUEST_TTL_SECS,
};
use std::{
    io,
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};
use uuid::Uuid;
use winit::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoop},
};

#[derive(Debug, Parser)]
#[command(name = "silo-tray", about = "Silo menu-bar and system-tray companion")]
struct Args {
    #[arg(short, long, default_value = "silo.vault")]
    vault: PathBuf,
    #[arg(long, default_value_t = silo_protocol::DEFAULT_SESSION_TIMEOUT_SECS)]
    timeout: u64,
    #[arg(long)]
    cli: Option<PathBuf>,
}

const STATUS_ID: &str = "status";
const UNLOCK_ID: &str = "unlock";
const SHELL_ID: &str = "shell";
const LOCK_ID: &str = "lock";
const QUIT_ID: &str = "quit";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let broker = silo_broker::start_locked(args.vault.clone(), args.timeout)?;
    let cli = args
        .cli
        .or_else(|| std::env::var_os("SILO_BIN").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("silo"));
    run_tray(broker, args.vault, cli)
}

#[allow(deprecated)]
fn run_tray(
    broker: BrokerHandle,
    vault: PathBuf,
    cli: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let menu = Menu::new();
    let status = MenuItem::with_id(STATUS_ID, "Silo · Locked", false, None);
    let unlock = MenuItem::with_id(UNLOCK_ID, "Unlock Silo", true, None);
    let shell = MenuItem::with_id(SHELL_ID, "Open shell", true, None);
    let lock = MenuItem::with_id(LOCK_ID, "Lock Silo", false, None);
    let quit = MenuItem::with_id(QUIT_ID, "Quit Silo", true, None);
    menu.append(&status)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&unlock)?;
    menu.append(&shell)?;
    menu.append(&lock)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit)?;

    let mut tray: Option<TrayIcon> = None;
    let mut last_status = None;
    let menu_receiver = MenuEvent::receiver();

    event_loop.run(move |event, event_loop| {
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(500),
        ));
        match event {
            Event::NewEvents(StartCause::Init) => {
                tray = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(menu.clone()))
                        .with_tooltip("Silo · local vault")
                        .with_icon(icon())
                        .with_menu_on_left_click(true)
                        .with_menu_on_right_click(true)
                        .build()
                        .expect("could not create Silo tray icon"),
                );
            }
            Event::AboutToWait => {
                let unlocked = broker_status().unwrap_or(false);
                if last_status != Some(unlocked) {
                    last_status = Some(unlocked);
                    let label = if unlocked {
                        "Silo · Unlocked"
                    } else {
                        "Silo · Locked"
                    };
                    status.set_text(label);
                    lock.set_enabled(unlocked);
                    unlock.set_enabled(!unlocked);
                    if let Some(tray) = tray.as_ref() {
                        let _ = tray.set_tooltip(Some(if unlocked {
                            "Silo · unlocked"
                        } else {
                            "Silo · locked"
                        }));
                    }
                }
                while let Ok(event) = menu_receiver.try_recv() {
                    match event.id().0.as_str() {
                        UNLOCK_ID => {
                            #[cfg(target_os = "macos")]
                            {
                                let _ = unlock_with_native_dialog();
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                let _ = open_terminal_command(&cli, &vault, "unlock");
                            }
                        }
                        SHELL_ID => {
                            let _ = open_terminal_command(&cli, &vault, "shell");
                        }
                        LOCK_ID => broker.lock(),
                        QUIT_ID => event_loop.exit(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    })?;
    Ok(())
}

fn broker_status() -> io::Result<bool> {
    let state = read_state(broker_state_path())
        .map_err(|error| io::Error::new(io::ErrorKind::NotConnected, error))?;
    let mut stream = TcpStream::connect(state.address)?;
    let envelope = Envelope {
        version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        expires_at: now().saturating_add(REQUEST_TTL_SECS),
        token: state.token,
        request: Request::Status,
    };
    write_frame(
        &mut stream,
        &serde_json::to_vec(&envelope).map_err(io::Error::other)?,
    )?;
    let Some(bytes) = read_frame(&mut stream)? else {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "broker returned no status",
        ));
    };
    let response: Response = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    Ok(response.unlocked == Some(true))
}

#[cfg(target_os = "macos")]
fn unlock_with_native_dialog() -> io::Result<()> {
    let script = r#"
set resultRecord to display dialog "Unlock Silo" with title "Silo" default answer "" buttons {"Cancel", "Unlock"} default button "Unlock" cancel button "Cancel" with hidden answer
return text returned of resultRecord
"#;
    let output = Command::new("osascript").args(["-e", script]).output()?;
    if !output.status.success() {
        return Ok(());
    }
    let password = String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if password.is_empty() {
        return Ok(());
    }
    let response = broker_request(Request::Unlock { password })?;
    if response.ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            response
                .error
                .clone()
                .unwrap_or_else(|| "could not unlock Silo".into()),
        ))
    }
}

#[cfg(target_os = "macos")]
fn broker_request(request: Request) -> io::Result<Response> {
    let state = read_state(broker_state_path())
        .map_err(|error| io::Error::new(io::ErrorKind::NotConnected, error))?;
    let mut stream = TcpStream::connect(state.address)?;
    let envelope = Envelope {
        version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        expires_at: now().saturating_add(REQUEST_TTL_SECS),
        token: state.token,
        request,
    };
    write_frame(
        &mut stream,
        &serde_json::to_vec(&envelope).map_err(io::Error::other)?,
    )?;
    let Some(bytes) = read_frame(&mut stream)? else {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "broker returned no response",
        ));
    };
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn open_terminal_command(
    cli: &Path,
    vault: &Path,
    subcommand: &str,
) -> io::Result<std::process::Child> {
    #[cfg(target_os = "macos")]
    {
        let command = format!(
            "tell application \"Terminal\" to do script {}",
            applescript_string(&format!(
                "{} --vault {} {}",
                shell_quote(cli),
                shell_quote(vault),
                subcommand
            ))
        );
        return Command::new("osascript").args(["-e", &command]).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("cmd")
            .args([
                "/C",
                "start",
                "Silo",
                &cli.to_string_lossy(),
                "--vault",
                &vault.to_string_lossy(),
                subcommand,
            ])
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        return Command::new("x-terminal-emulator")
            .args([
                "-e",
                &cli.to_string_lossy(),
                "--vault",
                &vault.to_string_lossy(),
                subcommand,
            ])
            .stdin(Stdio::null())
            .spawn();
    }

    #[allow(unreachable_code)]
    Command::new(cli)
        .args(["--vault", &vault.to_string_lossy(), subcommand])
        .stdin(Stdio::null())
        .spawn()
}

#[cfg(target_os = "macos")]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn applescript_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn icon() -> Icon {
    let size = 18u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let edge = x == 0 || y == 0 || x == size - 1 || y == size - 1;
            let mark = (7..=10).contains(&x) && (3..=14).contains(&y);
            if edge || mark {
                let index = ((y * size + x) * 4) as usize;
                rgba[index..index + 4].copy_from_slice(&[184, 242, 203, 255]);
            }
        }
    }
    Icon::from_rgba(rgba, size, size).expect("valid Silo icon")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
