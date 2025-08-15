use std::path::PathBuf;
use turbocache::TurboCache;

const HOME_DIR: &str = "vuoto_cli";

fn main() {
    let home_dir = get_app_dir();

    // exit w/ error
    if home_dir.is_none() {
        eprintln!("Unable to read HOMEDIR");
        std::process::exit(1);
    }

    let cache_path = home_dir.unwrap().join("vault");
    let _cache = TurboCache::new(cache_path, 512).expect("Cache init");
}

fn get_app_dir() -> Option<PathBuf> {
    if let Some(path) = env_home::env_home_dir() {
        return Some(path.join(&HOME_DIR));
    }

    None
}
