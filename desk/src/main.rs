//! Platform wrapper for the registration server.
//!
//! The server process has no GUI. This binary is the per-platform desk:
//! it starts the server in the background and shows a tray icon.
//! Linux, macOS, and Windows each have their own module.

mod server;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

fn main() {
    if let Err(err) = run() {
        eprintln!("rust-reg-desk: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    linux::run()
}

#[cfg(target_os = "macos")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    macos::run()
}

#[cfg(target_os = "windows")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    windows::run()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("reg-server desk has no wrapper for this platform".into())
}
