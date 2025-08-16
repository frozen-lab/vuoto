#![allow(dead_code)]

mod types;
mod vaults;

use crate::{
    types::{InternalError, InternalResult},
    vaults::VaultIndex,
};
use base64::{engine::general_purpose, Engine as _};
use inquire::{Select, Text};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use turbocache::TurboCache;

const HOME_DIR: &str = "vuoto_cli";

fn main() -> InternalResult<()> {
    let home_dir = get_app_dir()?;
    let vault = vault_selection_loop(&home_dir)?;

    login_selection_loop(&home_dir, &vault)?;

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct LoginEntry {
    name: String,
    password: String,
    username: Option<String>,
    url: Option<String>,
}

fn login_selection_loop<P: AsRef<Path>>(home_dir: &P, vault: &str) -> InternalResult<()> {
    let cache_path = home_dir.as_ref().join(vault);
    let cache = TurboCache::new(cache_path, 512)?;

    loop {
        let mut options: Vec<String> = Vec::new();

        // collect existing entries
        for i in cache.iter()? {
            let (key, value) = i?;

            let decoded: String = String::from_utf8(key.clone())
                .unwrap_or_else(|_| format!("(invalid key: {:?})", key));
            let entry: LoginEntry = serde_json::from_slice(&value).unwrap_or(LoginEntry {
                name: decoded.clone(),
                password: "".into(),
                username: None,
                url: None,
            });

            options.push(entry.name);
        }

        // menu options
        options.insert(0, "< Create new entry >".into());

        // let user pick
        let ans = Select::new("Your entries:", options).prompt();

        match ans {
            Ok(choice) => {
                if choice == "< Create new entry >" {
                    let entry = prompt_new_entry()?;

                    let key = general_purpose::STANDARD.encode(&entry.name);
                    let val = serde_json::to_vec(&entry).map_err(|e| {
                        InternalError::IO(format!("Failed to serialize entry: {e}"))
                    })?;

                    cache.set(key.as_bytes(), &val)?;

                    // loop again so new entry appears in list
                    continue;
                } else {
                    // fetch and show details
                    let key = general_purpose::STANDARD.encode(&choice);

                    if let Some(val) = cache.get(key.as_bytes())? {
                        let entry: LoginEntry = serde_json::from_slice(&val).map_err(|e| {
                            InternalError::IO(format!("Failed to decode entry: {e}"))
                        })?;

                        println!("\n=== Entry Details ===");
                        println!("Name: {}", entry.name);

                        if let Some(u) = entry.username {
                            println!("Username: {}", u);
                        }

                        println!("Password: {}", entry.password);

                        if let Some(u) = entry.url {
                            println!("URL: {}", u);
                        }

                        println!("=====================\n");
                    } else {
                        eprintln!("Entry not found!");
                    }

                    // exit app after showing one
                    break;
                }
            }

            Err(err) => return_error(format!("{err}")),
        }
    }

    Ok(())
}

fn prompt_new_entry() -> InternalResult<LoginEntry> {
    let name = Text::new("Entry name:")
        .prompt()
        .map_err(|e| InternalError::IO(format!("Failed to read input: {e}")))?;

    let username = Text::new("Username (optional):")
        .prompt_skippable()
        .map_err(|e| InternalError::IO(format!("Failed to read username: {e}")))?;

    let password = Text::new("Password:")
        .prompt()
        .map_err(|e| InternalError::IO(format!("Failed to read password: {e}")))?;

    let url = Text::new("URL (optional):")
        .prompt_skippable()
        .map_err(|e| InternalError::IO(format!("Failed to read URL: {e}")))?;

    Ok(LoginEntry {
        name,
        password,
        username,
        url,
    })
}

fn vault_selection_loop<P: AsRef<Path>>(home_dir: &P) -> InternalResult<String> {
    let vault = loop {
        let mut vault_idx = VaultIndex::open(&home_dir.as_ref())?;
        let mut options = vault_idx.vaults().to_vec();

        // no vaults yet => force creation
        if options.is_empty() {
            let new_vault = prompt_new_vault()?;
            vault_idx.add(&new_vault)?;

            break new_vault;
        } else {
            // add special option
            options.push("< Create new vault >".into());

            let ans = Select::new("Your Vaults:", options).prompt();

            match ans {
                Ok(choice) => {
                    if choice == "< Create new vault >" {
                        let new_vault = prompt_new_vault()?;
                        vault_idx.add(&new_vault)?;

                        continue;
                    } else {
                        break choice;
                    }
                }

                Err(err) => return_error(format!("{err}")),
            }
        }
    };

    Ok(vault)
}

fn prompt_new_vault() -> InternalResult<String> {
    let ans = Text::new("Enter name for new vault:")
        .prompt()
        .map_err(|e| InternalError::IO(format!("Failed to read input: {e}")))?;

    Ok(ans)
}

fn return_error(msg: String) -> ! {
    eprintln!("[ERROR]: {msg}");

    std::process::exit(1);
}

pub(crate) fn get_app_dir() -> InternalResult<PathBuf> {
    let base = if cfg!(debug_assertions) {
        std::env::temp_dir()
    } else {
        env_home::env_home_dir()
            .ok_or_else(|| InternalError::IO("Unable to read home dir".into()))?
    };

    let dir = base.join(HOME_DIR);

    std::fs::create_dir_all(&dir)
        .map_err(|e| InternalError::IO(format!("Failed to create app dir: {}", e)))?;

    Ok(dir)
}
