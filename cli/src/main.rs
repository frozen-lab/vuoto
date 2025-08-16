#![allow(dead_code)]

mod types;
mod vaults;

use crate::{
    types::{InternalError, InternalResult},
    vaults::VaultIndex,
};
use inquire::{Select, Text};
use std::path::PathBuf;
use turbocache::TurboCache;

const HOME_DIR: &str = "vuoto_cli";

fn main() -> InternalResult<()> {
    let home_dir = get_app_dir()?;
    let cache_path = home_dir.join("vault");
    let _cache = TurboCache::new(cache_path, 512)?;
    let mut vault_idx = VaultIndex::open(&home_dir)?;

    let vault = loop {
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

    println!("Vault selected: {}", vault);
    Ok(())
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
