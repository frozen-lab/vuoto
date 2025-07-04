use clap::{Arg, Command};
use turbocache::TurboCache;

fn main() -> std::io::Result<()> {
    let mut cache = TurboCache::open("/home/adii/")?;

    let matches = Command::new("passwd_app")
        .version("0.1.0")
        .author("Your Name <you@example.com>")
        .about("Manage user passwords via CLI")
        // get
        .arg(
            Arg::new("get")
                .long("get")
                .help("Retrieve and print the stored password")
                .num_args(1)
                .value_name("USERNAME"),
        )
        // set
        .arg(
            Arg::new("set")
                .long("set")
                .help("Set password for a user: provide username and password")
                .num_args(2)
                .value_names(["USERNAME", "PASSWORD"]),
        )
        .get_matches();

    // handle get
    if let Some(username) = matches.get_one::<String>("get") {
        let key = String::from(username).into_bytes();
        if let Some(val) = cache.get(&key)? {
            let pass = String::from_utf8(val).unwrap();

            println!("Password for {}: {}", username, pass);
            return Ok(());
        }

        eprintln!("Error: user '{}' not found", username);
        std::process::exit(1);
    }

    // handle set
    if let Some(values) = matches.get_many::<String>("set") {
        let args: Vec<&String> = values.collect();
        let username = String::from(args[0]).into_bytes();
        let password = String::from(args[1]).into_bytes();

        if username.is_empty() || password.is_empty() {
            eprintln!("Error: username and password must be non-empty");
            std::process::exit(1);
        }

        cache.set(&username, &password)?;
        return Ok(());
    }

    eprintln!("No operation specified. Use --help for usage information.");
    std::process::exit(1);
}
