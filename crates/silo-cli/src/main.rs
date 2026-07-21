use arboard::Clipboard;
use clap::{Parser, Subcommand, ValueEnum};
mod tui;
use rand::Rng;
use silo_core::{
    generate_totp, inspect_totp, load_vault, new_entry, save_vault, Entry, SecretString, Vault,
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use time::OffsetDateTime;
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
    Init,
    Shell {
        #[arg(long, default_value_t = 900)]
        timeout: u64,
    },
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
    List,
    Show {
        query: String,
    },
    Get {
        query: String,
        #[arg(value_enum, default_value_t = Field::Password)]
        field: Field,
    },
    Copy {
        query: String,
        #[arg(value_enum, default_value_t = Field::Password)]
        field: Field,
        #[arg(short, long, default_value_t = 20)]
        seconds: u64,
    },
    Otp {
        query: String,
    },
    OtpCheck {
        query: String,
    },
    SetTotp {
        query: String,
        secret: Option<String>,
    },
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
    Remove {
        query: String,
        #[arg(short, long)]
        yes: bool,
    },
    Generate {
        #[arg(short, long, default_value_t = 24)]
        length: usize,
    },
    Export {
        output: PathBuf,
    },
    Import {
        input: PathBuf,
        #[arg(long)]
        replace: bool,
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
        Command::Import { input, replace } => import_vault(&cli.vault, &input, replace)?,
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
        entry.password = SecretString::new(prompt_password("New entry password: ")?.to_string());
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
    let plaintext = Zeroizing::new(serde_json::to_vec_pretty(&vault)?);
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
) -> Result<(), Box<dyn std::error::Error>> {
    let imported: Vault = serde_json::from_slice(&fs::read(input)?)?;
    let (mut vault, master) = unlock(path)?;
    if replace {
        vault = imported;
    } else {
        for entry in imported.entries {
            vault.add(entry);
        }
    }
    save_vault(path, &vault, &master)?;
    println!("Imported entries from {}.", input.display());
    Ok(())
}

fn run_shell(path: &Path, timeout: u64) -> Result<(), Box<dyn std::error::Error>> {
    tui::run(path, timeout)
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
