#![allow(dead_code)]

mod types;
mod vaults;

use crate::types::{InternalError, InternalResult};
use std::path::PathBuf;
use turbocache::TurboCache;

const HOME_DIR: &str = "vuoto_cli";

fn main() -> InternalResult<()> {
    let home_dir = get_app_dir()?;
    let cache_path = home_dir.join("vault");
    let _cache = TurboCache::new(cache_path, 512)?;

    Ok(())
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
