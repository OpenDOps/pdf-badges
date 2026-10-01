//! Manual check against the real registration server.
//!
//! `cargo test` skips this. Pass the account on the command line:
//!
//! ```text
//! cargo test --test live_remote -- --email you@example.com --password secret --device-id device-1
//! ```
//!
//! `--expo` also calls `profile/synchdev` and writes the project token. That
//! attaches this device id to that exhibition on the remote server.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use rust_reg::registration_server::{remote_base_from_env, CredentialFile, Remote, REMOTE_ENV};

#[derive(Parser)]
#[command(name = "live_remote")]
struct LiveArgs {
    /// Operator email
    #[arg(long)]
    email: Option<String>,

    /// Operator password
    #[arg(long)]
    password: Option<String>,

    /// Provisioned device id sent to profile/synchdev
    #[arg(long)]
    device_id: Option<String>,

    /// Host or URL. Unset uses REGISTRATION_REMOTE, or kuprin.su.
    #[arg(long)]
    remote: Option<String>,

    /// Page size for boxapi/expos/ru
    #[arg(long, default_value_t = 20)]
    limit: u64,

    /// Offset for boxapi/expos/ru
    #[arg(long, default_value_t = 0)]
    skip: u64,

    /// Exhibition id to bind. Omit this to only log in and list.
    #[arg(long)]
    expo: Option<String>,

    /// Display name stored with --expo
    #[arg(long)]
    expo_name: Option<String>,

    /// Credential file written when --expo is set
    #[arg(long)]
    out: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = LiveArgs::parse();
    if args.email.is_none()
        && args.password.is_none()
        && args.device_id.is_none()
        && args.expo.is_none()
    {
        println!(
            "live remote skipped. To call the server:\n  cargo test --test live_remote -- --email EMAIL --password PASSWORD --device-id DEVICE"
        );
        return ExitCode::SUCCESS;
    }

    let (Some(email), Some(password), Some(device_id)) =
        (args.email, args.password, args.device_id)
    else {
        eprintln!("--email, --password, and --device-id are required together");
        return ExitCode::FAILURE;
    };
    if args.expo.is_some() && args.out.is_none() {
        eprintln!("--expo writes a project token; pass --out path/to/credentials.json");
        return ExitCode::FAILURE;
    }

    let remote = match &args.remote {
        Some(raw) => Remote::new(raw),
        None => remote_base_from_env().and_then(|url| Remote::new(url.as_str())),
    };
    let remote = match remote {
        Ok(remote) => remote,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    println!("remote {}", remote.base());
    if args.remote.is_none() {
        println!("host from {REMOTE_ENV}, default kuprin.su");
    }

    let session = match remote.login(&device_id, &email, &password).await {
        Ok(session) => session,
        Err(err) => {
            eprintln!("login failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("logged in as {email} for device {device_id}");

    let exhibitions = match session.list_exhibitions(args.limit, args.skip).await {
        Ok(exhibitions) => exhibitions,
        Err(err) => {
            eprintln!("list failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("{} exhibitions", exhibitions.len());
    for exhibition in &exhibitions {
        if exhibition.name.is_empty() {
            println!("  {}", exhibition.unique_id);
        } else {
            println!("  {}  {}", exhibition.unique_id, exhibition.name);
        }
    }

    let Some(expo_id) = args.expo else {
        return ExitCode::SUCCESS;
    };
    let out = args.out.expect("--out is required with --expo");
    let file = CredentialFile::new(&out);
    if let Err(err) = session.bind(&file, &expo_id, args.expo_name).await {
        eprintln!("bind failed: {err}");
        return ExitCode::FAILURE;
    }
    match file.load() {
        Ok(stored) if stored.is_logged_in() => {
            println!("bound {expo_id}; project token stored in {}", out.display());
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("bind returned but the credential file is not logged in");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
