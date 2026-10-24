//! Shared process control for the platform desk wrappers.
//!
//! Installed copies live in `<prefix>/bin` next to `rust-reg-run` and start
//! that supervisor. A development copy starts the sibling `rust-reg` binary.
//! The macOS application bundle always uses `/Library/rust-reg`.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const DEFAULT_PORT: u16 = 8080;

pub enum Opened {
    Ready(Session),
    AlreadyRunning,
}

pub struct Session {
    child: Option<Child>,
    address: Arc<Mutex<String>>,
    _lock: File,
}

pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Session {
    pub fn open() -> io::Result<Opened> {
        let lock = match lock_file()? {
            Some(file) => file,
            None => return Ok(Opened::AlreadyRunning),
        };
        let address = Arc::new(Mutex::new(format!("localhost:{DEFAULT_PORT}")));
        let child = if port_open(DEFAULT_PORT) {
            None
        } else {
            Some(spawn_server(Arc::clone(&address))?)
        };
        Ok(Opened::Ready(Session {
            child,
            address,
            _lock: lock,
        }))
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn address(&self) -> &Arc<Mutex<String>> {
        &self.address
    }

    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub fn address_text(&self) -> String {
        self.address.lock().expect("address").clone()
    }

    /// Stop the process this desk started, or the installed service when the
    /// port was already taken by that service.
    pub fn stop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            stop_child(child);
            return;
        }
        stop_installed_service();
    }
}

pub(crate) fn launch_for(exe: &Path) -> Launch {
    let (run_name, server_name) = binary_names(exe);
    if app_bundle(exe) {
        let prefix = PathBuf::from("/Library/rust-reg");
        return Launch {
            program: prefix.join("bin").join(run_name),
            args: prefix_args(&prefix),
        };
    }
    if let Some(dir) = exe.parent() {
        if dir.file_name().and_then(|name| name.to_str()) == Some("bin") {
            if let Some(prefix) = dir.parent() {
                return Launch {
                    program: dir.join(run_name),
                    args: prefix_args(prefix),
                };
            }
        }
        return Launch {
            program: dir.join(server_name),
            args: Vec::new(),
        };
    }
    Launch {
        program: PathBuf::from(server_name),
        args: Vec::new(),
    }
}

pub(crate) fn desk_address(line: &str) -> Option<String> {
    let rest = line.split("http+ws ").nth(1)?;
    let listen = rest.split_whitespace().next()?;
    let (host, port) = listen.rsplit_once(':')?;
    if port.parse::<u16>().is_err() {
        return None;
    }
    let host = host.trim_matches(|ch| ch == '[' || ch == ']');
    let host = if host.is_empty() || host == "0.0.0.0" || host == "::" {
        "localhost"
    } else {
        host
    };
    Some(format!("{host}:{port}"))
}

/// Template menu-bar mark. Only the pencil and the person are opaque.
/// macOS paints that shape in the menu-bar color and leaves the rest clear.
/// The menu bar fits the image to 22 points, so the glyphs are drawn 1.3× larger
/// inside a fixed height and the width grows to keep both of them visible.
pub const ICON_WIDTH: u32 = 148;
pub const ICON_HEIGHT: u32 = 128;
const MARK_SCALE: f32 = 1.3;
const SRC_CX: f32 = 61.0;
const SRC_CY: f32 = 65.5;

pub fn icon_rgba() -> (u32, u32, Vec<u8>) {
    icon_from_pencil(None)
}

/// Same mark, with `pencil` (top-down alpha, one byte per output pixel) in place
/// of the drawn fallback. macOS passes the system pencil glyph here.
pub fn icon_with_pencil(pencil: &[u8]) -> (u32, u32, Vec<u8>) {
    icon_from_pencil(Some(pencil))
}

/// AppKit draw rect (origin at the bottom left) for the system pencil, in the output image.
pub fn pencil_symbol_rect() -> (f64, f64, f64, f64) {
    let left = map_x(4.0);
    let right = map_x(68.0);
    let top = map_y(18.0);
    let bottom = map_y(110.0);
    (
        left as f64,
        (ICON_HEIGHT as f32 - bottom) as f64,
        (right - left) as f64,
        (bottom - top) as f64,
    )
}

fn map_x(src: f32) -> f32 {
    ICON_WIDTH as f32 / 2.0 + (src - SRC_CX) * MARK_SCALE
}

fn map_y(src: f32) -> f32 {
    ICON_HEIGHT as f32 / 2.0 + (src - SRC_CY) * MARK_SCALE
}

fn source_point(x: u32, y: u32) -> (f32, f32) {
    let sx = SRC_CX + (x as f32 + 0.5 - ICON_WIDTH as f32 / 2.0) / MARK_SCALE;
    let sy = SRC_CY + (y as f32 + 0.5 - ICON_HEIGHT as f32 / 2.0) / MARK_SCALE;
    (sx, sy)
}

fn icon_from_pencil(pencil: Option<&[u8]>) -> (u32, u32, Vec<u8>) {
    let pixels = (ICON_WIDTH * ICON_HEIGHT) as usize;
    let mut data = vec![0u8; pixels * 4];
    let mask = pencil.filter(|mask| mask.len() == pixels);
    for y in 0..ICON_HEIGHT {
        for x in 0..ICON_WIDTH {
            let (px, py) = source_point(x, y);
            let index = (y * ICON_WIDTH + x) as usize;
            let drawn = mask
                .map(|mask| mask[index] as f32 / 255.0)
                .unwrap_or_else(|| pencil_coverage(px, py));
            let glyph = drawn.max(person_coverage(px, py));
            data[index * 4 + 3] = (glyph.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    (ICON_WIDTH, ICON_HEIGHT, data)
}

fn cover(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

fn sd_round_box(px: f32, py: f32, cx: f32, cy: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let dx = (px - cx).abs() - half_w + radius;
    let dy = (py - cy).abs() - half_h + radius;
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius
}

fn sd_circle(px: f32, py: f32, cx: f32, cy: f32, radius: f32) -> f32 {
    (px - cx).hypot(py - cy) - radius
}

fn sd_capsule(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, radius: f32) -> f32 {
    let (pax, pay) = (px - ax, py - ay);
    let (bax, bay) = (bx - ax, by - ay);
    let len = bax * bax + bay * bay;
    let along = if len == 0.0 {
        0.0
    } else {
        ((pax * bax + pay * bay) / len).clamp(0.0, 1.0)
    };
    (pax - bax * along).hypot(pay - bay * along) - radius
}

fn sd_polygon(px: f32, py: f32, points: &[(f32, f32)]) -> f32 {
    let mut closest = {
        let dx = px - points[0].0;
        let dy = py - points[0].1;
        dx * dx + dy * dy
    };
    let mut sign = 1.0f32;
    let mut previous = points.len() - 1;
    for index in 0..points.len() {
        let (ex, ey) = (
            points[previous].0 - points[index].0,
            points[previous].1 - points[index].1,
        );
        let (wx, wy) = (px - points[index].0, py - points[index].1);
        let len = ex * ex + ey * ey;
        let along = if len == 0.0 {
            0.0
        } else {
            ((wx * ex + wy * ey) / len).clamp(0.0, 1.0)
        };
        let (bx, by) = (wx - ex * along, wy - ey * along);
        closest = closest.min(bx * bx + by * by);
        let above_here = py >= points[index].1;
        let below_previous = py < points[previous].1;
        let left_of_edge = ex * wy > ey * wx;
        if (above_here && below_previous && left_of_edge)
            || (!above_here && !below_previous && !left_of_edge)
        {
            sign = -sign;
        }
        previous = index;
    }
    sign * closest.sqrt()
}

fn pencil_coverage(x: f32, y: f32) -> f32 {
    let body = sd_polygon(
        x,
        y,
        &[
            (18.0, 100.0),
            (30.0, 74.0),
            (42.0, 52.0),
            (50.0, 34.0),
            (66.0, 44.0),
            (58.0, 58.0),
            (46.0, 80.0),
            (34.0, 96.0),
        ],
    );
    let band = sd_capsule(x, y, 44.0, 46.0, 60.0, 58.0, 1.7);
    (cover(body) - cover(band)).max(0.0)
}

fn person_coverage(x: f32, y: f32) -> f32 {
    let head = sd_circle(x, y, 92.0, 46.0, 14.0);
    let body = sd_round_box(x, y, 92.0, 86.0, 22.0, 20.0, 14.0);
    cover(head.min(body))
}

fn spawn_server(address: Arc<Mutex<String>>) -> io::Result<Child> {
    let exe = std::env::current_exe()?;
    let launch = launch_for(&exe);
    if !launch.program.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("missing {}", launch.program.display()),
        ));
    }
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || watch_address(stderr, address));
    }
    Ok(child)
}

fn watch_address(stderr: impl std::io::Read, address: Arc<Mutex<String>>) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        if let Some(shown) = desk_address(&line) {
            *address.lock().expect("address") = shown;
        }
    }
}

fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(50)),
                Err(_) => return,
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn stop_installed_service() {
    #[cfg(target_os = "linux")]
    crate::linux::stop_installed_service();
    #[cfg(target_os = "macos")]
    crate::macos::stop_installed_service();
    #[cfg(target_os = "windows")]
    crate::windows::stop_installed_service();
}

fn lock_file() -> io::Result<Option<File>> {
    let path = lock_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(err)) => return Err(err),
    }
    file.set_len(0)?;
    writeln!(file, "{}", std::process::id())?;
    Ok(Some(file))
}

fn lock_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    dir.join("rust-reg-desk.lock")
}

fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

fn prefix_args(prefix: &Path) -> Vec<String> {
    vec!["--prefix".to_string(), prefix.display().to_string()]
}

fn binary_names(exe: &Path) -> (&'static str, &'static str) {
    if cfg!(windows) || exe.extension().and_then(|ext| ext.to_str()) == Some("exe") {
        ("rust-reg-run.exe", "rust-reg.exe")
    } else {
        ("rust-reg-run", "rust-reg")
    }
}

fn app_bundle(exe: &Path) -> bool {
    exe.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.ends_with(".app"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspecified_host_is_localhost() {
        assert_eq!(
            desk_address("registration-server http+ws 0.0.0.0:8080"),
            Some("localhost:8080".to_string())
        );
        assert_eq!(
            desk_address("registration-server http+ws [::]:9090"),
            Some("localhost:9090".to_string())
        );
    }

    #[test]
    fn specific_host_is_kept() {
        assert_eq!(
            desk_address("registration-server http+ws 10.0.0.8:8080"),
            Some("10.0.0.8:8080".to_string())
        );
    }

    #[test]
    fn installed_bin_starts_the_supervisor() {
        let launch = launch_for(Path::new("/opt/rust-reg/bin/rust-reg-desk"));
        assert_eq!(launch.program, Path::new("/opt/rust-reg/bin/rust-reg-run"));
        assert_eq!(
            launch.args,
            vec!["--prefix".to_string(), "/opt/rust-reg".to_string()]
        );
    }

    #[test]
    fn windows_bin_uses_exe_names() {
        let launch = launch_for(Path::new("/Program Files/rust-reg/bin/rust-reg-desk.exe"));
        assert_eq!(
            launch.program,
            Path::new("/Program Files/rust-reg/bin/rust-reg-run.exe")
        );
        assert_eq!(
            launch.args,
            vec![
                "--prefix".to_string(),
                "/Program Files/rust-reg".to_string()
            ]
        );
    }

    #[test]
    fn mac_app_uses_the_library_prefix() {
        let launch = launch_for(Path::new(
            "/Applications/reg-server.app/Contents/MacOS/reg-server",
        ));
        assert_eq!(
            launch.program,
            Path::new("/Library/rust-reg/bin/rust-reg-run")
        );
        assert_eq!(
            launch.args,
            vec!["--prefix".to_string(), "/Library/rust-reg".to_string()]
        );
    }

    #[test]
    fn development_copy_starts_the_sibling_server() {
        let launch = launch_for(Path::new("/tmp/target/debug/rust-reg-desk"));
        assert_eq!(launch.program, Path::new("/tmp/target/debug/rust-reg"));
        assert!(launch.args.is_empty());
    }
}
