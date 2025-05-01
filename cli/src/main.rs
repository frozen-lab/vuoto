use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{Password, theme::ColorfulTheme};
use directories::ProjectDirs;
use hmac::{Hmac, Mac};
use indicatif::{ProgressBar, ProgressStyle};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{collections::HashMap, fs, io::Write, path::PathBuf, time::Duration};

/// Vuoto CLI: seed-based password generator and store
#[derive(Parser)]
#[command(name = "vuoto_cli", version, author)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Skip the spinner animation
    #[arg(long)]
    no_spinner: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a password for an app (and store it)
    Generate {
        /// Optional username
        #[arg(short, long)]
        username: Option<String>,
        /// App name (key)
        #[arg(short, long)]
        app: String,
        /// Expiry seed (e.g. y2025, m7, d15)
        #[arg(short, long)]
        exp: Option<String>,
    },
    /// Retrieve a stored password by app name
    Get {
        #[arg(short, long)]
        app: String,
    },
    /// Reset seed and all stored passwords
    Reset,
}

#[derive(Serialize, Deserialize, Default)]
struct StoredPasswords {
    map: HashMap<String, String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    println!("{}", style("🔐 Vuoto CLI").bold().cyan());

    if let Commands::Reset = &cli.command {
        reset_all()?;
        println!("{}", style("All data reset.").green());
        return Ok(());
    }

    let phrase = retrieve_or_prompt()?;
    let seed = preprocess(&phrase);

    match cli.command {
        Commands::Generate { username, app, exp } => {
            animate("Generating password...", !cli.no_spinner);
            let meta = format!(
                "{}|{}|{}",
                username.unwrap_or_default(),
                app,
                exp.unwrap_or_default()
            );
            let pwd = generate_password(&seed, &meta);
            store_password(&app, &pwd)?;
            println!(
                "Password for {}: {}",
                style(&app).cyan(),
                style(&pwd).yellow()
            );
        }
        Commands::Get { app } => {
            animate("Fetching password...", !cli.no_spinner);
            let pwd = retrieve_password(&app)?;
            println!(
                "Password for {}: {}",
                style(&app).cyan(),
                style(&pwd).green()
            );
        }
        Commands::Reset => unreachable!(),
    }

    Ok(())
}

fn preprocess(input: &str) -> String {
    input
        .trim()
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
        .replace(' ', "-")
}

fn animate(msg: &str, spinner_on: bool) {
    if spinner_on {
        let sp = ProgressBar::new_spinner();
        sp.set_style(
            ProgressStyle::with_template("{spinner:.magenta} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        sp.enable_steady_tick(Duration::from_millis(80));
        // use owned String to satisfy 'static requirement
        sp.set_message(msg.to_string());
        std::thread::sleep(Duration::from_millis(500));
        sp.finish_and_clear();
    }
}

fn generate_password(seed: &str, meta: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(seed.as_bytes()).unwrap();
    mac.update(meta.as_bytes());
    let result = mac.finalize().into_bytes();
    let charset: Vec<char> =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()_+"
            .chars()
            .collect();
    result
        .iter()
        .take(16)
        .map(|b| charset[(*b as usize) % charset.len()])
        .collect()
}

fn retrieve_or_prompt() -> Result<String> {
    let kr = Entry::new("vuoto_cli_seed", "default").context("Keyring init failed")?;
    match kr.get_password() {
        Ok(p) => Ok(p),
        Err(e) if e.to_string().contains("No such object") => prompt_and_store(&kr),
        Err(e) if e.to_string().contains("Failed to connect to socket") => fallback_prompt(),
        Err(e) => {
            eprintln!("{} {}", style("Warn:").yellow(), e);
            fallback_prompt()
        }
    }
}

fn prompt_and_store(kr: &Entry) -> Result<String> {
    let p = Password::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter a secret seed phrase (20+ chars)")
        .with_confirmation("Confirm", "Mismatch")
        .interact()?;
    if p.len() < 20 {
        anyhow::bail!("Seed phrase must be 20+ characters");
    }
    kr.set_password(&p)?;
    println!("{}", style("Seed saved securely 🎉").green());
    Ok(p)
}

fn fallback_prompt() -> Result<String> {
    let proj = ProjectDirs::from("com", "example", "vuoto_cli").context("Config dir failed")?;
    let mut path: PathBuf = proj.config_dir().to_path_buf();
    fs::create_dir_all(&path)?;
    path.push("seed.txt");
    if path.exists() {
        Ok(fs::read_to_string(&path)?)
    } else {
        let p = Password::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter a secret seed phrase (20+ chars)")
            .with_confirmation("Confirm", "Mismatch")
            .interact()?;
        if p.len() < 20 {
            anyhow::bail!("Seed must be 20+ chars");
        }
        fs::write(&path, &p)?;
        println!("{} {}", style("Seed saved to").green(), path.display());
        Ok(p)
    }
}

fn store_password(app: &str, pwd: &str) -> Result<()> {
    let kr = Entry::new("vuoto_cli_pw", app)?;
    match kr.set_password(pwd) {
        Ok(_) => return Ok(()),
        Err(e) if e.to_string().contains("Failed to connect") => (),
        Err(e) => anyhow::bail!(e),
    }
    let mut store = load_json()?;
    store.map.insert(app.to_string(), pwd.to_string());
    save_json(&store)
}

fn retrieve_password(app: &str) -> Result<String> {
    let kr = Entry::new("vuoto_cli_pw", app)?;
    if let Ok(p) = kr.get_password() {
        return Ok(p);
    }
    let store = load_json()?;
    store.map.get(app).cloned().context("Password not found")
}

fn load_json() -> Result<StoredPasswords> {
    let proj = ProjectDirs::from("com", "example", "vuoto_cli").unwrap();
    let mut path: PathBuf = proj.config_dir().to_path_buf();
    fs::create_dir_all(&path)?;
    path.push("passwords.json");
    if path.exists() {
        let data = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(StoredPasswords::default())
    }
}

fn save_json(store: &StoredPasswords) -> Result<()> {
    let proj = ProjectDirs::from("com", "example", "vuoto_cli").unwrap();
    let mut path: PathBuf = proj.config_dir().to_path_buf();
    path.push("passwords.json");
    let mut file = fs::File::create(&path)?;
    file.write_all(serde_json::to_string_pretty(&store)?.as_bytes())?;
    Ok(())
}

fn reset_all() -> Result<()> {
    // delete seed from keyring and file
    if let Ok(kr) = Entry::new("vuoto_cli_seed", "default") {
        let _ = kr.delete_credential();
    }
    if let Some(proj) = ProjectDirs::from("com", "example", "vuoto_cli") {
        let dir: PathBuf = proj.config_dir().to_path_buf();
        let _ = fs::remove_file(dir.join("seed.txt"));
        let _ = fs::remove_file(dir.join("passwords.json"));
    }
    Ok(())
}
