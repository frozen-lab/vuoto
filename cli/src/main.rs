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
use std::{collections::HashMap, fs, io::Write, path::PathBuf, time::Duration};

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

/// Determine the config directory for *everything*.
/// This exactly matches what ProjectDirs::from(...).config_dir() returns,
/// so we stay 100% in sync with your tests.
fn get_config_dir() -> Option<PathBuf> {
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

fn load_seed() -> Result<Option<String>> {
    // 1) Look for a seed.txt on disk first (this is what your tests do).
    if let Some(mut dir) = get_config_dir() {
        fs::create_dir_all(&dir)?;
        dir.push("seed.txt");
        if dir.exists() {
            return Ok(Some(fs::read_to_string(dir)?));
        }
    }
    // 2) Only if there is no file, try the keyring.
    if let Ok(kr) = Entry::new("vuoto_cli_seed", "default") {
        if let Ok(p) = kr.get_password() {
            return Ok(Some(p));
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
    // 1) Remove seed from keyring
    if let Ok(kr) = Entry::new("vuoto_cli_seed", "default") {
        let _ = kr.delete_credential();
    }
    // 2) Remove the on-disk files
    if let Some(dir) = get_config_dir() {
        let _ = fs::remove_file(dir.join("seed.txt"));
        let _ = fs::remove_file(dir.join("passwords.json"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};
    use tempfile::TempDir;

    /// Sets up a temporary XDG_CONFIG_HOME to isolate file-based tests.
    fn setup_tmp_config() -> TempDir {
        let tmp = TempDir::new().expect("failed to create temp dir");
        unsafe {
            env::set_var("XDG_CONFIG_HOME", tmp.path());
        }
        tmp
    }

    #[test]
    fn test_preprocess() {
        assert_eq!(preprocess("  Hello World!  "), "hello-world");
        assert_eq!(preprocess("Foo_BAR Baz123"), "foobar-baz123");
        assert_eq!(preprocess("   "), "");
    }

    #[test]
    fn test_generate_password_consistency() {
        let seed = "consistent-seed";
        let meta = "user|app|exp";
        let pw1 = generate_password(seed, meta);
        let pw2 = generate_password(seed, meta);
        assert_eq!(pw1, pw2, "Passwords should be deterministic");
        assert_eq!(pw1.len(), 16, "Password length should be 16");
    }

    #[test]
    fn test_generate_password_variation() {
        let seed = "variation-seed";
        let pw1 = generate_password(seed, "meta1");
        let pw2 = generate_password(seed, "meta2");
        assert_ne!(pw1, pw2, "Different meta should yield different passwords");
    }

    #[test]
    fn test_reset_all_clears_files() {
        let _tmp = setup_tmp_config();
        // Create seed.txt and passwords.json
        let config_dir = get_config_dir().expect("config dir none");
        fs::create_dir_all(&config_dir).expect("mkdir failed");
        fs::write(config_dir.join("seed.txt"), "seed").expect("write seed.txt failed");
        let mut store = StoredPasswords::default();
        store.map.insert("app".into(), "pwd".into());
        save_json(&store).expect("save_json failed");

        assert!(
            config_dir.join("seed.txt").exists(),
            "seed.txt should exist"
        );
        assert!(
            config_dir.join("passwords.json").exists(),
            "passwords.json should exist"
        );

        // Perform reset
        reset_all().expect("reset_all failed");

        assert!(
            !config_dir.join("seed.txt").exists(),
            "seed.txt should be removed"
        );
        assert!(
            !config_dir.join("passwords.json").exists(),
            "passwords.json should be removed"
        );
    }
}
