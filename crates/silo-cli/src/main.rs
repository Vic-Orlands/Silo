use arboard::Clipboard;
use clap::{Parser, Subcommand, ValueEnum};
mod tui;
use rand::Rng;
use silo_core::{
    export_json, generate_totp, import_migration, inspect_totp, load_vault, new_entry, save_vault,
    Entry, ImportFormat, MigrationPreview, SecretString, Vault,
};
use std::{
    fs,
    io::{self, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use time::OffsetDateTime;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Debug, Parser)]
#[command(
    name = "silo",
    version,
    about = "A local-first password manager that keeps your vault on this computer"
)]
struct Cli {
    #[arg(short, long, global = true, default_value = "silo.vault")]
    vault: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new encrypted vault
    Init,
    /// Open the interactive terminal workspace
    Shell {
        #[arg(long, default_value_t = silo_protocol::DEFAULT_SESSION_TIMEOUT_SECS)]
        timeout: u64,
    },
    /// Run the local browser session broker
    Broker {
        #[arg(long, default_value_t = silo_protocol::DEFAULT_SESSION_TIMEOUT_SECS)]
        timeout: u64,
        #[arg(long)]
        background: bool,
    },
    /// Unlock a running background broker
    Unlock,
    /// Lock a running background broker
    Lock,
    /// Show the background broker state
    Status,
    /// Add a login to the vault
    Add {
        name: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        username: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long, conflicts_with = "password_file")]
        password: Option<String>,
        #[arg(long = "password-file", conflicts_with = "password")]
        password_file: Option<PathBuf>,
        #[arg(long = "totp-secret")]
        totp_secret: Option<String>,
    },
    /// List the logins in the vault
    List,
    /// Show login metadata without revealing its password
    Show { query: String },
    /// Print a field from a login
    Get {
        query: String,
        #[arg(value_enum, default_value_t = Field::Password)]
        field: Field,
    },
    /// Copy a login field and clear it from the clipboard later
    Copy {
        query: String,
        #[arg(value_enum, default_value_t = Field::Password)]
        field: Field,
        #[arg(short, long, default_value_t = 20)]
        seconds: u64,
    },
    /// Generate the current TOTP code for a login
    Otp { query: String },
    /// Diagnose the TOTP configuration for a login
    OtpCheck { query: String },
    /// Add, replace, or clear a login's TOTP secret
    SetTotp {
        query: String,
        secret: Option<String>,
    },
    /// Edit an existing login
    Edit {
        query: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        username: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        password: bool,
    },
    /// Remove a login from the vault
    Remove {
        query: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// Generate a password without saving it
    Generate {
        #[arg(short, long, default_value_t = 24)]
        length: usize,
    },
    /// Export the vault to a plaintext JSON file
    Export { output: PathBuf },
    /// Import logins from a supported export file
    Import {
        input: PathBuf,
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        expect_count: Option<usize>,
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Field {
    Username,
    Email,
    Password,
    Url,
}

struct AddOptions {
    url: Option<String>,
    username: Option<String>,
    email: Option<String>,
    password: Option<String>,
    password_file: Option<PathBuf>,
    totp_secret: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => init(&cli.vault)?,
        Command::Shell { timeout } => run_shell(&cli.vault, timeout)?,
        Command::Broker {
            timeout,
            background,
        } => {
            if background {
                silo_broker::run_background(cli.vault, timeout)?;
            } else {
                silo_broker::run_with_config(cli.vault, timeout)?;
            }
        }
        Command::Unlock => unlock_broker()?,
        Command::Lock => lock_broker()?,
        Command::Status => status_broker()?,
        Command::Add {
            name,
            url,
            username,
            email,
            password,
            password_file,
            totp_secret,
        } => add(
            &cli.vault,
            name,
            AddOptions {
                url,
                username,
                email,
                password,
                password_file,
                totp_secret,
            },
        )?,
        Command::List => list(&cli.vault)?,
        Command::Show { query } => show(&cli.vault, &query)?,
        Command::Get { query, field } => print_field(&cli.vault, &query, field)?,
        Command::Copy {
            query,
            field,
            seconds,
        } => copy_field(&cli.vault, &query, field, seconds)?,
        Command::Otp { query } => otp(&cli.vault, &query)?,
        Command::OtpCheck { query } => otp_check(&cli.vault, &query)?,
        Command::SetTotp { query, secret } => set_totp(&cli.vault, query, secret)?,
        Command::Edit {
            query,
            name,
            url,
            username,
            email,
            password,
        } => edit(&cli.vault, query, name, url, username, email, password)?,
        Command::Remove { query, yes } => remove(&cli.vault, query, yes)?,
        Command::Generate { length } => println!("{}", generate_password(length)?),
        Command::Export { output } => export_vault(&cli.vault, &output)?,
        Command::Import {
            input,
            replace,
            dry_run,
            expect_count,
            format,
        } => import_vault(&cli.vault, &input, replace, dry_run, expect_count, format)?,
    }
    Ok(())
}

fn init(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        return Err("vault already exists".into());
    }
    let password = prompt_new_password()?;
    save_vault(path, &Vault::new(), &password)?;
    println!("Created encrypted Silo at {}", path.display());
    Ok(())
}

fn add(path: &Path, name: String, options: AddOptions) -> Result<(), Box<dyn std::error::Error>> {
    let master = prompt_password("Silo password: ")?;
    let mut vault = load_vault(path, &master)?;
    let url = match options.url {
        Some(value) => value,
        None => prompt_line("URL: ")?,
    };
    let username = match options.username {
        Some(value) => value,
        None => prompt_line("Username: ")?,
    };
    let email = match options.email {
        Some(value) => value,
        None => prompt_line("Email (optional): ")?,
    };
    let password = match (options.password, options.password_file) {
        (Some(value), None) => Zeroizing::new(value),
        (None, Some(path)) => Zeroizing::new(fs::read_to_string(path)?.trim_end().to_string()),
        (None, None) => prompt_password("Entry password: ")?,
        (Some(_), Some(_)) => unreachable!("clap prevents both password sources"),
    };
    let totp_secret = match options.totp_secret {
        Some(secret) => {
            let secret = secret.trim().to_string();
            inspect_totp(&secret)?;
            Some(secret)
        }
        None => optional_secret()?,
    };
    vault.add(new_entry(
        name,
        url,
        username,
        email,
        password.to_string(),
        totp_secret,
    ));
    save_vault(path, &vault, &master)?;
    println!("Entry saved.");
    Ok(())
}

fn list(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let vault = unlock(path)?.0;
    if vault.entries.is_empty() {
        println!("Your Silo is empty. Add one with: silo add <name>");
    } else {
        for entry in vault.entries {
            println!("{}\t{}\t{}", entry.name, entry.username, entry.url);
        }
    }
    Ok(())
}

fn show(path: &Path, query: &str) -> Result<(), Box<dyn std::error::Error>> {
    let vault = unlock(path)?.0;
    let entry = vault.find(query).ok_or("entry not found")?;
    print_metadata(entry);
    Ok(())
}

fn print_field(path: &Path, query: &str, field: Field) -> Result<(), Box<dyn std::error::Error>> {
    let vault = unlock(path)?.0;
    let entry = vault.find(query).ok_or("entry not found")?;
    println!("{}", field_value(entry, field));
    Ok(())
}

fn copy_field(
    path: &Path,
    query: &str,
    field: Field,
    seconds: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let vault = unlock(path)?.0;
    let entry = vault.find(query).ok_or("entry not found")?;
    copy_value(field_value(entry, field), seconds)
}

fn otp(path: &Path, query: &str) -> Result<(), Box<dyn std::error::Error>> {
    let vault = unlock(path)?.0;
    let entry = vault.find(query).ok_or("entry not found")?;
    let secret = entry
        .totp_secret
        .as_ref()
        .ok_or("this entry has no TOTP secret; use silo set-totp <name>")?;
    println!("{}", generate_totp(secret.as_str(), now())?);
    Ok(())
}

fn otp_check(path: &Path, query: &str) -> Result<(), Box<dyn std::error::Error>> {
    let vault = unlock(path)?.0;
    let entry = vault.find(query).ok_or("entry not found")?;
    let secret = entry
        .totp_secret
        .as_ref()
        .ok_or("this entry has no TOTP secret; use silo set-totp <name>")?;
    let metadata = inspect_totp(secret.as_str())?;
    let timestamp = now();
    println!("Entry:       {}", entry.name);
    println!("Source:      {}", metadata.source);
    println!("Algorithm:   {}", metadata.algorithm);
    println!("Digits:      {}", metadata.digits);
    println!("Period:      {} seconds", metadata.period);
    println!("Secret:      valid ({} bytes)", metadata.secret_bytes);
    println!(
        "Current OTP: {}",
        generate_totp(secret.as_str(), timestamp)?
    );
    println!(
        "Refreshes:   {} seconds",
        metadata.period - timestamp % metadata.period
    );
    Ok(())
}

fn set_totp(
    path: &Path,
    query: String,
    secret: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut vault, master) = unlock(path)?;
    let entry = vault.find_mut(&query).ok_or("entry not found")?;
    let secret = match secret {
        Some(secret) => secret,
        None => prompt_line("TOTP secret or otpauth URI (blank clears it): ")?,
    };
    if !secret.trim().is_empty() {
        inspect_totp(secret.trim())?;
    }
    entry.totp_secret = (!secret.trim().is_empty()).then_some(SecretString::new(secret.trim()));
    let name = entry.name.clone();
    save_vault(path, &vault, &master)?;
    println!("TOTP updated for {name}.");
    Ok(())
}

fn edit(
    path: &Path,
    query: String,
    name: Option<String>,
    url: Option<String>,
    username: Option<String>,
    email: Option<String>,
    change_password: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut vault, master) = unlock(path)?;
    let entry = vault.find_mut(&query).ok_or("entry not found")?;
    if let Some(name) = name {
        entry.name = name;
    }
    if let Some(url) = url {
        entry.url = url;
    }
    if let Some(username) = username {
        entry.username = username;
    }
    if let Some(email) = email {
        entry.email = email;
    }
    if change_password {
        let password = prompt_password("New entry password: ")?;
        entry.password = SecretString::new(password.as_str());
    }
    save_vault(path, &vault, &master)?;
    println!("Entry updated.");
    Ok(())
}

fn remove(path: &Path, query: String, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (mut vault, master) = unlock(path)?;
    let entry = vault.find(query.as_str()).ok_or("entry not found")?;
    if !yes
        && prompt_line(&format!(
            "Delete '{}'? Type 'yes' to continue: ",
            entry.name
        ))? != "yes"
    {
        println!("Cancelled.");
        return Ok(());
    }
    vault.remove(&query).ok_or("entry disappeared")?;
    save_vault(path, &vault, &master)?;
    println!("Entry deleted.");
    Ok(())
}

fn export_vault(path: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let vault = unlock(path)?.0;
    let plaintext = Zeroizing::new(export_json(&vault)?);
    fs::write(output, plaintext.as_slice())?;
    set_private_permissions(output)?;
    println!(
        "Exported plaintext JSON to {}. Protect or delete this file.",
        output.display()
    );
    Ok(())
}

fn import_vault(
    path: &Path,
    input: &Path,
    replace: bool,
    dry_run: bool,
    expect_count: Option<usize>,
    format: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(input)?;
    let requested = match format.as_deref() {
        None => ImportFormat::Auto,
        Some(value) => ImportFormat::parse(value).ok_or(
            "unsupported import format; use auto, silo-json, bitwarden-json, 1password-csv, keepass-csv, browser-csv, or csv",
        )?,
    };
    let (mut vault, master) = unlock(path)?;
    let comparison_vault = if replace { Vault::new() } else { vault.clone() };
    let preview = import_migration(&bytes, requested, &comparison_vault)?;
    print_migration_report(input, &preview);
    if let Some(expected) = expect_count {
        if expected != preview.entries.len() {
            return Err(format!(
                "expected {expected} valid entries, found {}",
                preview.entries.len()
            )
            .into());
        }
    }
    if dry_run {
        println!("Dry run only; the vault was not changed.");
        return Ok(());
    }
    let duplicates = preview
        .duplicate_indices
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let imported = preview
        .entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| (!duplicates.contains(&index)).then_some(entry))
        .collect::<Vec<_>>();
    if replace {
        vault = Vault { entries: imported };
    } else {
        for entry in imported {
            vault.add(entry);
        }
    }
    save_vault(path, &vault, &master)?;
    println!("Imported entries from {}.", input.display());
    Ok(())
}

fn print_migration_report(path: &Path, preview: &MigrationPreview) {
    println!("Import:          {}", path.display());
    println!("Detected format: {}", preview.format.label());
    println!("Valid entries:   {}", preview.entries.len());
    println!("Duplicates:      {}", preview.duplicate_indices.len());
    println!("Failed rows:     {}", preview.issues.len());
    for issue in &preview.issues {
        println!("  Row {}: {}", issue.row, issue.message);
    }
}

fn run_shell(path: &Path, timeout: u64) -> Result<(), Box<dyn std::error::Error>> {
    tui::run(path, timeout)
}

fn unlock_broker() -> Result<(), Box<dyn std::error::Error>> {
    let password = prompt_password("Silo password: ")?;
    let response = broker_request(silo_protocol::Request::Unlock {
        password: silo_protocol::SensitiveString::new(password.as_str()),
    })?;
    if !response.ok {
        return Err(response
            .error
            .clone()
            .unwrap_or_else(|| "could not unlock Silo".into())
            .into());
    }
    println!("Silo is unlocked for this session.");
    Ok(())
}

fn lock_broker() -> Result<(), Box<dyn std::error::Error>> {
    let response = broker_request(silo_protocol::Request::Lock)?;
    if !response.ok {
        return Err(response
            .error
            .clone()
            .unwrap_or_else(|| "could not lock Silo".into())
            .into());
    }
    println!("Silo is locked.");
    Ok(())
}

fn status_broker() -> Result<(), Box<dyn std::error::Error>> {
    let response = broker_request(silo_protocol::Request::Status)?;
    if !response.ok {
        return Err(response
            .error
            .clone()
            .unwrap_or_else(|| "could not read Silo status".into())
            .into());
    }
    println!(
        "Silo broker: {}",
        if response.unlocked == Some(true) {
            "unlocked"
        } else {
            "locked"
        }
    );
    Ok(())
}

fn broker_request(
    request: silo_protocol::Request,
) -> Result<silo_protocol::Response, Box<dyn std::error::Error>> {
    let state = silo_protocol::read_state(silo_protocol::broker_state_path())
        .map_err(|_| "Silo broker is not running; start `silo broker --background` first")?;
    let mut stream =
        TcpStream::connect(&state.address).map_err(|_| "Silo broker is not reachable")?;
    let envelope = silo_protocol::Envelope {
        version: silo_protocol::PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        expires_at: unix_now().saturating_add(silo_protocol::REQUEST_TTL_SECS),
        token: state.token,
        request,
    };
    let encoded = serde_json::to_vec(&envelope)?;
    silo_protocol::write_frame(&mut stream, &encoded)?;
    let Some(bytes) = silo_protocol::read_frame(&mut stream)? else {
        return Err("Silo broker returned no response".into());
    };
    Ok(serde_json::from_slice(&bytes)?)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unlock(path: &Path) -> Result<(Vault, Zeroizing<String>), Box<dyn std::error::Error>> {
    let master = prompt_password("Silo password: ")?;
    let vault = load_vault(path, &master)?;
    Ok((vault, master))
}

fn print_metadata(entry: &Entry) {
    println!("Name:     {}", entry.name);
    println!("Username: {}", entry.username);
    println!("Email:    {}", entry.email);
    println!("URL:      {}", entry.url);
    println!(
        "TOTP:     {}",
        if entry.totp_secret.is_some() {
            "configured"
        } else {
            "not configured"
        }
    );
}

fn field_value(entry: &Entry, field: Field) -> &str {
    match field {
        Field::Username => &entry.username,
        Field::Email => &entry.email,
        Field::Password => entry.password.as_str(),
        Field::Url => &entry.url,
    }
}

fn copy_value(value: &str, seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
    if !(1..=300).contains(&seconds) {
        return Err("clipboard timeout must be between 1 and 300 seconds".into());
    }
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(value)?;
    println!("Copied. Clearing clipboard in {seconds} seconds.");
    thread::sleep(Duration::from_secs(seconds));
    if clipboard.get_text().ok().as_deref() == Some(value) {
        clipboard.set_text("")?;
    } else {
        println!("Clipboard changed by another application; leaving it untouched.");
        return Ok(());
    }
    println!("Clipboard cleared.");
    Ok(())
}

fn generate_password(length: usize) -> Result<String, Box<dyn std::error::Error>> {
    if !(12..=128).contains(&length) {
        return Err("password length must be between 12 and 128".into());
    }
    let alphabet = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";
    let mut rng = rand::thread_rng();
    let mut output = String::with_capacity(length);
    for _ in 0..length {
        output.push(alphabet[rng.gen_range(0..alphabet.len())] as char);
    }
    Ok(output)
}

fn optional_secret() -> Result<Option<String>, Box<dyn std::error::Error>> {
    let secret = prompt_line("TOTP secret or otpauth URI (press Enter to skip): ")?;
    if !secret.trim().is_empty() {
        inspect_totp(secret.trim())?;
    }
    Ok((!secret.trim().is_empty()).then_some(secret.trim().to_string()))
}

fn prompt_new_password() -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    let first = prompt_password("Create Silo password: ")?;
    let second = prompt_password("Repeat Silo password: ")?;
    if first.is_empty() || *first != *second {
        return Err("passwords are empty or do not match".into());
    }
    Ok(first)
}

fn prompt_password(prompt: &str) -> Result<Zeroizing<String>, io::Error> {
    print!("{prompt}");
    io::stdout().flush()?;
    Ok(Zeroizing::new(rpassword::read_password()?))
}

fn prompt_line(prompt: &str) -> Result<String, io::Error> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_string())
}

fn now() -> u64 {
    OffsetDateTime::now_utc().unix_timestamp().max(0) as u64
}

fn set_private_permissions(path: &Path) -> Result<(), io::Error> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
