mod auth;
mod demo;
mod mcp;
mod web;
use std::{io, path::Path};
use yy_accountant::Store;

fn main() {
    if let Err(error) = run() {
        eprintln!("yyaccountant: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let text = |i: usize| args.get(i).and_then(|s| s.to_str());
    match text(0) {
        Some("--version") => println!("yyaccountant {}", env!("CARGO_PKG_VERSION")),
        Some("--mcp") => {
            if !(2..=3).contains(&args.len()) || (args.len() == 3 && text(2) != Some("--read-only"))
            {
                return Err("Usage: yyaccountant --mcp DATABASE [--read-only]".into());
            }
            let mut store = Store::open(Path::new(&args[1]), args.len() == 3)?;
            mcp::serve(&mut store, io::stdin().lock(), io::stdout().lock())?;
        }
        Some("--web") => {
            let path = args.get(1).ok_or(
                "Usage: yyaccountant --web DATABASE [--listen IP:PORT] [--auth-config FILE] [--read-only]",
            )?;
            let mut listen = "127.0.0.1:8750".parse()?;
            let mut config = std::path::PathBuf::from("yyaccountant-server.toml");
            let mut read_only = false;
            let mut i = 2;
            while i < args.len() {
                match text(i) {
                    Some("--listen") => {
                        i += 1;
                        listen = text(i).ok_or("Missing listen IP:PORT")?.parse()?;
                    }
                    Some("--auth-config") => {
                        i += 1;
                        config = args.get(i).ok_or("Missing auth config path")?.into();
                    }
                    Some("--read-only") => read_only = true,
                    _ => return Err("Unknown Web server option".into()),
                }
                i += 1;
            }
            let store = Store::open(Path::new(path), read_only)?;
            web::serve(store, listen, config, read_only)?;
        }
        Some("--generate-password") => {
            let user = text(1).ok_or(
                "Usage: yyaccountant --generate-password USER [--auth-config FILE] [--read-only]",
            )?;
            let mut config = std::path::PathBuf::from("yyaccountant-server.toml");
            let mut read_only = false;
            let mut i = 2;
            while i < args.len() {
                match text(i) {
                    Some("--auth-config") => {
                        i += 1;
                        config = args.get(i).ok_or("Missing auth config path")?.into();
                    }
                    Some("--read-only") => read_only = true,
                    _ => return Err("Unknown credential option".into()),
                }
                i += 1;
            }
            let password =
                auth::generate_password(&config, user, read_only).map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::json!({"user":user,"password":password,"config":config,"read_only":read_only})
            );
        }
        Some("--init-demo") => {
            let usage = "Usage: yyaccountant --init-demo NEW_DATABASE [--today YYYY-MM-DD]";
            let path = args.get(1).ok_or(usage)?;
            let today = match (text(2), text(3)) {
                (None, None) => chrono::Local::now().date_naive(),
                (Some("--today"), Some(d)) if args.len() == 4 => d.parse()?,
                _ => return Err(usage.into()),
            };
            demo::create(Path::new(path), today)?;
            println!("Created {}", Path::new(path).display());
        }
        Some("--backup") => {
            if args.len() != 3 {
                return Err("Usage: yyaccountant --backup DATABASE NEW_FILE".into());
            }
            Store::open(Path::new(&args[1]), true)?.backup(Path::new(&args[2]))?;
            println!("Backed up to {}", Path::new(&args[2]).display());
        }
        Some("--help") | None => print!("{}", include_str!("../../help/help.md")),
        _ => return Err("Unknown option; use --help".into()),
    }
    Ok(())
}
