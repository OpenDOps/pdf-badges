//! Platform wrapper for the registration server.
//!
//! The server process has no GUI. This binary is the per-platform desk:
//! it starts the server in the background and shows a tray icon.
//! Linux, macOS, and Windows each have their own module.
//!
//! A launch from a terminal returns the prompt. The icon keeps running
//! in its own session.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod server;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

fn main() {
    if let Some((path, edge)) = write_icon_arg() {
        let (width, height, rgba) = tray_icon();
        let (size, square) = server::square_rgba(width, height, &rgba);
        let (out_w, out_h, pixels) = match edge {
            Some(edge) if edge > 0 && edge != size => {
                server::scale_rgba(size, size, &square, edge, edge)
            }
            _ => (size, size, square),
        };
        if let Err(err) = server::write_png(&path, out_w, out_h, &pixels) {
            eprintln!("rust-reg-desk: {err}");
            std::process::exit(1);
        }
        return;
    }
    // Log before any detach so Applications-menu failures are visible.
    let tty_stdin = stdin_is_tty();
    let from_desktop = std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE")
        .or_else(|| std::env::var_os("DESKTOP_STARTUP_ID"))
        .is_some();
    server::desk_log(format!(
        "main start pid={} desktop={} tty_stdin={} no_detach={}",
        std::process::id(),
        from_desktop,
        tty_stdin,
        std::env::var_os("RUST_REG_DESK_NO_DETACH").is_some()
    ));
    detach_from_terminal();
    if let Err(err) = run() {
        server::desk_log(format!("main exit error: {err}"));
        eprintln!("rust-reg-desk: {err}");
        std::process::exit(1);
    }
    server::desk_log("main exit ok");
}

#[cfg(target_os = "macos")]
fn tray_icon() -> (u32, u32, Vec<u8>) {
    macos::menu_icon()
}

#[cfg(not(target_os = "macos"))]
fn tray_icon() -> (u32, u32, Vec<u8>) {
    server::icon_rgba()
}

fn stdin_is_tty() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::isatty(libc::STDIN_FILENO) != 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn write_icon_arg() -> Option<(std::path::PathBuf, Option<u32>)> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--write-icon") {
        return None;
    }
    let Some(path) = args.next() else {
        eprintln!("usage: rust-reg-desk --write-icon <file.png> [edge]");
        std::process::exit(1);
    };
    let edge = args.next().and_then(|text| text.parse().ok());
    Some((std::path::PathBuf::from(path), edge))
}

/// Leave the controlling terminal when one is attached. A menu launcher has
/// no terminal, and this returns without forking.
///
/// Never fork when started from a `.desktop` / GNOME Activities launch: the
/// parent would exit inside a systemd scope and GNOME kills the child (looks
/// like an instant crash), while a terminal launch reparents the child to init
/// and keeps working.
#[cfg(unix)]
fn detach_from_terminal() {
    if std::env::var_os("RUST_REG_DESK_NO_DETACH").is_some()
        || std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE").is_some()
        || std::env::var_os("DESKTOP_STARTUP_ID").is_some()
    {
        server::desk_log("detach skipped (desktop / NO_DETACH)");
        return;
    }
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 0 {
        return;
    }
    let mut pipe = [0; 2];
    if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
        return;
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        unsafe {
            libc::close(pipe[0]);
            libc::close(pipe[1]);
        }
        return;
    }
    if pid > 0 {
        unsafe {
            libc::close(pipe[1]);
            let mut byte = [0u8; 1];
            libc::read(pipe[0], byte.as_mut_ptr().cast(), 1);
            libc::_exit(0);
        }
    }
    unsafe {
        libc::close(pipe[0]);
        libc::setsid();
        libc::close(pipe[1]);
    }
    server::desk_log(format!("detached child pid={}", std::process::id()));
    redirect_stdio();
}

#[cfg(unix)]
fn redirect_stdio() {
    let path = std::ffi::CString::new("/dev/null").expect("/dev/null");
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return;
    }
    for target in 0..3 {
        unsafe { libc::dup2(fd, target) };
    }
    if fd > 2 {
        unsafe { libc::close(fd) };
    }
}

/// A console launch exits after starting a copy with no console, so `cmd`
/// gets its prompt back. The copy is marked and does not start another.
#[cfg(windows)]
fn detach_from_terminal() {
    if std::env::var_os("RUST_REG_DESK_DETACHED").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    let spawned = std::process::Command::new(exe)
        .args(std::env::args().skip(1))
        .env("RUST_REG_DESK_DETACHED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn();
    if spawned.is_ok() {
        std::process::exit(0);
    }
}

#[cfg(not(any(unix, windows)))]
fn detach_from_terminal() {}

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
