use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{Password, theme::ColorfulTheme};
use directories::ProjectDirs;
use hmac::{Hmac, Mac};
use indicatif::{ProgressBar, ProgressStyle};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{collections::HashMap, env, fs, io::Write, path::PathBuf, time::Duration};

/// Vuoto CLI: seed-based password generator and store
#[derive(Parser)]
#[command(name = "vuoto_cli", version, author)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Skip spinner animation
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
    /// Reset seed and all stored data
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

    // Load or prompt seed
    let seed = match load_seed()? {
        Some(s) => s,
        None => prompt_and_store_seed()?,
    };
    let processed_seed = preprocess(&seed);

    match cli.command {
        Commands::Generate { username, app, exp } => {
            animate("Generating password...", !cli.no_spinner);
            let meta = format!(
                "{}|{}|{}",
                username.unwrap_or_default(),
                app,
                exp.unwrap_or_default()
            );
            let pwd = generate_password(&processed_seed, &meta);
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

/// Determine the config directory, respecting XDG_CONFIG_HOME if set
fn get_config_dir() -> Option<PathBuf> {
    if let Some(x) = env::var_os("XDG_CONFIG_HOME") {
        let mut p = PathBuf::from(x);
        p.push("com");
        p.push("example");
        p.push("vuoto_cli");
        return Some(p);
    }
    ProjectDirs::from("com", "example", "vuoto_cli").map(|proj| proj.config_dir().to_path_buf())
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

/// Attempts to load seed from keyring or file
fn load_seed() -> Result<Option<String>> {
    // Keyring
    if let Ok(kr) = Entry::new("vuoto_cli_seed", "default") {
        if let Ok(p) = kr.get_password() {
            return Ok(Some(p));
        }
    }
    // Fallback file
    if let Some(mut dir) = get_config_dir() {
        fs::create_dir_all(&dir)?;
        dir.push("seed.txt");
        if dir.exists() {
            return Ok(Some(fs::read_to_string(dir)?));
        }
    }
    Ok(None)
}

/// Prompt user for seed and store in both keyring and file
fn prompt_and_store_seed() -> Result<String> {
    let phrase = Password::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter a secret seed phrase (20+ chars)")
        .with_confirmation("Confirm", "Mismatch")
        .interact()?;
    if phrase.len() < 20 {
        bail!("Seed phrase must be 20+ characters");
    }
    // Store in keyring (ignore errors)
    if let Ok(kr) = Entry::new("vuoto_cli_seed", "default") {
        let _ = kr.set_password(&phrase);
    }
    // Store to file
    if let Some(mut dir) = get_config_dir() {
        fs::create_dir_all(&dir)?;
        dir.push("seed.txt");
        fs::write(dir, &phrase)?;
    }
    println!("{}", style("Seed saved securely 🎉").green());
    Ok(phrase)
}

/// Stores password in keyring; falls back to JSON file
fn store_password(app: &str, pwd: &str) -> Result<()> {
    if let Ok(kr) = Entry::new("vuoto_cli_pw", app) {
        if kr.set_password(pwd).is_ok() {
            return Ok(());
        }
    }
    // JSON fallback
    let mut store = load_json()?;
    store.map.insert(app.to_string(), pwd.to_string());
    save_json(&store)
}

/// Retrieves password from keyring or JSON
fn retrieve_password(app: &str) -> Result<String> {
    if let Ok(kr) = Entry::new("vuoto_cli_pw", app) {
        if let Ok(p) = kr.get_password() {
            return Ok(p);
        }
    }
    let store = load_json()?;
    store.map.get(app).cloned().context("Password not found")
}

fn load_json() -> Result<StoredPasswords> {
    if let Some(mut dir) = get_config_dir() {
        fs::create_dir_all(&dir)?;
        dir.push("passwords.json");
        if dir.exists() {
            let data = fs::read_to_string(&dir)?;
            return Ok(serde_json::from_str(&data)?);
        }
    }
    Ok(StoredPasswords::default())
}

fn save_json(store: &StoredPasswords) -> Result<()> {
    if let Some(mut dir) = get_config_dir() {
        fs::create_dir_all(&dir)?;
        dir.push("passwords.json");
        let mut file = fs::File::create(&dir)?;
        file.write_all(serde_json::to_string_pretty(&store)?.as_bytes())?;
    }
    Ok(())
}

fn reset_all() -> Result<()> {
    // Remove seed
    if let Ok(kr) = Entry::new("vuoto_cli_seed", "default") {
        let _ = kr.delete_credential();
    }
    // Remove fallback files (seed.txt and passwords.json)
    if let Some(proj) = ProjectDirs::from("com", "example", "vuoto_cli") {
        let dir = proj.config_dir();
        let _ = fs::remove_file(dir.join("seed.txt"));
        let _ = fs::remove_file(dir.join("passwords.json"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_preprocess_basic() {
        assert_eq!(
            preprocess("Hello, Rust World 2025!"),
            "hello-rust-world-2025"
        );
    }

    #[test]
    fn test_generate_password_deterministic_and_length() {
        let pwd1 = generate_password("seed", "meta");
        let pwd2 = generate_password("seed", "meta");
        assert_eq!(pwd1, pwd2);
        assert_eq!(pwd1.len(), 16);
    }

    #[test]
    fn test_save_and_load_json() {
        let dir = tempdir().unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }
        let mut store = StoredPasswords::default();
        store.map.insert("app1".into(), "pass1".into());
        save_json(&store).unwrap();
        let loaded = load_json().unwrap();
        assert_eq!(loaded.map.get("app1"), Some(&"pass1".into()));
    }

    #[test]
    fn test_fallback_seed_file() {
        let dir = tempdir().unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }
        let proj = ProjectDirs::from("com", "example", "vuoto_cli").unwrap();
        let mut path = proj.config_dir().to_path_buf();
        fs::create_dir_all(&path).unwrap();
        path.push("seed.txt");
        fs::write(&path, "mytestseed").unwrap();
        assert_eq!(load_seed().unwrap(), Some("mytestseed".into()));
    }

    #[test]
    fn test_store_and_retrieve_via_json() {
        let dir = tempdir().unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }
        let app = "appx";
        let pwd = "pwdx";
        store_password(app, pwd).unwrap();
        let got = retrieve_password(app).unwrap();
        assert_eq!(got, pwd);
    }

    #[test]
    fn test_reset_all_clears_files() {
        let dir = tempdir().unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }
        let proj = ProjectDirs::from("com", "example", "vuoto_cli").unwrap();
        let path = proj.config_dir().to_path_buf();
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("seed.txt"), "x").unwrap();
        fs::write(path.join("passwords.json"), "{}").unwrap();
        reset_all().unwrap();
        assert!(!path.join("seed.txt").exists());
        assert!(!path.join("passwords.json").exists());
    }
}
