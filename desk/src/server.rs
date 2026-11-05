//! Shared process control for the platform desk wrappers.
//!
//! Installed copies live in `<prefix>/bin` next to `rust-reg-run` and start
//! that supervisor. A development copy starts the sibling `rust-reg` binary.
//! The macOS application bundle always uses `/Library/rust-reg`.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
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
    port: u16,
    failure: Option<String>,
    /// Previous listen port while a Linux background switch is in flight.
    #[cfg(target_os = "linux")]
    inflight_old_port: Option<u16>,
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
        let mut session = Session {
            child: None,
            address: Arc::new(Mutex::new(String::new())),
            port: remembered_port(),
            failure: None,
            #[cfg(target_os = "linux")]
            inflight_old_port: None,
            _lock: lock,
        };
        // Persist (and migrate off the old runtime-dir path) on every launch.
        remember_port(session.port);
        session.begin();
        Ok(Opened::Ready(session))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Why the server is not running, when the last start failed.
    pub fn failure(&self) -> Option<String> {
        self.failure.clone()
    }

    /// Stop the current server, if this desk started one, and start on `port`.
    ///
    /// Does **not** touch the installed system service. May block for a few
    /// seconds while the old process exits — Linux Port apply runs
    /// [`restart_switch`] on a worker thread instead.
    pub fn restart(&mut self, port: u16) {
        desk_log(format!("restart {port} (was {})", self.port));
        self.stop_owned();
        self.port = port;
        remember_port(port);
        self.begin();
        desk_log(format!(
            "restart done → {} failure={:?}",
            self.address_text(),
            self.failure
        ));
    }

    /// Take the supervised child so a worker can stop it without holding the
    /// tray loop. Updates the remembered listen port immediately.
    #[cfg(target_os = "linux")]
    pub fn prepare_port_switch(&mut self, port: u16) -> (Option<Child>, u16) {
        let old_port = self.port;
        let child = self.child.take();
        desk_log(format!(
            "port switch prepare {old_port} → {port} (had_child={})",
            child.is_some()
        ));
        self.inflight_old_port = Some(old_port);
        self.port = port;
        remember_port(port);
        self.failure = None;
        *self.address.lock().expect("address") = format!("switching to {port}…");
        (child, old_port)
    }

    /// Apply the result of [`restart_switch`] on the tray thread.
    #[cfg(target_os = "linux")]
    pub fn finish_port_switch(
        &mut self,
        port: u16,
        child: Option<Child>,
        address: String,
        failure: Option<String>,
    ) {
        desk_log(format!(
            "port switch finish → {address} child={} failure={failure:?}",
            child.is_some()
        ));
        self.inflight_old_port = None;
        self.port = port;
        self.child = child;
        self.failure = failure;
        *self.address.lock().expect("address") = address;
    }

    /// Shared label for the Linux tray menu (ksni rebuilds from this mutex).
    #[cfg(target_os = "linux")]
    pub fn address(&self) -> &Arc<Mutex<String>> {
        &self.address
    }

    pub fn address_text(&self) -> String {
        self.address.lock().expect("address").clone()
    }

    /// Quit path: stop the child this desk started, then the installed service
    /// if it still holds the listen port, then any leftover listener.
    pub fn stop(&mut self) {
        desk_log(format!("stop begin port={}", self.port));
        self.stop_owned();
        // Only on Quit — never from Port restart (that blocked the Linux loop
        // on pkexec and made Quit / Port… appear dead).
        stop_installed_service();
        // Service stop may leave the listener briefly; clear again.
        stop_port(self.port);
        #[cfg(target_os = "linux")]
        if let Some(old) = self.inflight_old_port.take() {
            desk_log(format!("stop also clearing inflight old port {old}"));
            stop_port(old);
        }
        desk_log(format!(
            "stop done port={} still_open={}",
            self.port,
            port_open(self.port)
        ));
    }

    /// Stop only the process group this desk spawned (and clear that port).
    fn stop_owned(&mut self) {
        let port = self.port;
        if let Some(child) = self.child.take() {
            desk_log(format!("stop_owned child_pid={} port={port}", child.id()));
            stop_child(child);
        } else {
            desk_log(format!("stop_owned no child port={port}"));
        }
        // Orphans from an earlier tray (or a wrapper that respawned) may still
        // hold the listen port after the direct child is gone.
        stop_port(port);
    }

    fn begin(&mut self) {
        *self.address.lock().expect("address") = String::new();
        self.failure = None;
        // Service or leftover process already listening: show the address and
        // do not start a second server. Quit still clears the port via stop().
        if port_open(self.port) {
            let shown = format!("localhost:{}", self.port);
            desk_log(format!("begin adopt existing {shown}"));
            *self.address.lock().expect("address") = shown;
            return;
        }
        desk_log(format!("begin spawn port={}", self.port));
        match spawn_server(self.port, Arc::clone(&self.address)) {
            Ok(child) => {
                desk_log(format!(
                    "begin ok pid={} address={}",
                    child.id(),
                    self.address.lock().expect("address")
                ));
                self.child = Some(child);
            }
            Err(err) => {
                desk_log(format!("begin fail port={}: {err}", self.port));
                self.fail(err.to_string());
            }
        }
    }

    fn fail(&mut self, message: String) {
        self.child = None;
        self.failure = Some(message);
        *self.address.lock().expect("address") = format!("failed on port {}", self.port);
    }
}

/// Browser URL for a live listen line such as `localhost:8080`.
pub(crate) fn listen_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.starts_with("failed ") {
        return None;
    }
    let (host, port) = text.rsplit_once(':')?;
    if host.is_empty() || port.parse::<u16>().is_err() {
        return None;
    }
    Some(format!("http://{text}"))
}

/// Open the listen address in the default browser.
pub(crate) fn open_listen(text: &str) {
    let Some(url) = listen_url(text) else {
        return;
    };
    let mut command = {
        #[cfg(target_os = "macos")]
        {
            let mut command = Command::new("open");
            command.arg(&url);
            command
        }
        #[cfg(target_os = "linux")]
        {
            let mut command = Command::new("xdg-open");
            command.arg(&url);
            command
        }
        #[cfg(target_os = "windows")]
        {
            let mut command = Command::new("cmd");
            command.args(["/c", "start", "", &url]);
            command
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            let _ = url;
            return;
        }
    };
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// A different listen port, when `text` is a port other than `current`.
pub(crate) fn replacement_port(current: u16, text: &str) -> Option<u16> {
    let port = text.trim().parse::<u16>().ok()?;
    if port == 0 || port == current {
        None
    } else {
        Some(port)
    }
}

pub(crate) fn launch_for(exe: &Path) -> Launch {
    launch_for_in(exe, &support_prefix())
}

fn launch_for_in(exe: &Path, support: &Path) -> Launch {
    let (run_name, server_name) = binary_names(exe);
    if let Some(template) = bundled_template(exe, run_name) {
        let prefix = materialize(&template, support).unwrap_or(template);
        return Launch {
            program: prefix.join("bin").join(run_name),
            args: prefix_args(&prefix),
        };
    }
    if app_bundle(exe) {
        // Packaged installs put the tree in /Library/rust-reg. A drag-installed
        // thin app without that directory uses Application Support when a
        // previous run or DMG already materialized one there.
        let library = PathBuf::from("/Library/rust-reg");
        if library.join("bin").join(run_name).is_file() {
            return Launch {
                program: library.join("bin").join(run_name),
                args: prefix_args(&library),
            };
        }
        if support.join("bin").join(run_name).is_file() {
            return Launch {
                program: support.join("bin").join(run_name),
                args: prefix_args(support),
            };
        }
        return Launch {
            program: library.join("bin").join(run_name),
            args: prefix_args(&library),
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
    #[cfg(target_os = "macos")]
    {
        // Template: only alpha matters; the menu bar recolors the mark.
        return icon_from_pencil(None, [0, 0, 0]);
    }
    #[cfg(not(target_os = "macos"))]
    {
        // StatusNotifier / notification area show the pixmap as-is. Black
        // glyphs disappear on a dark panel; use the same blue as rust-reg.svg.
        icon_from_pencil([0x1f, 0x6f, 0xeb])
    }
}

/// Nearest-neighbor scale for StatusNotifier / hicolor PNG edges (hosts prefer ~22–48 px).
pub fn scale_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
    out_w: u32,
    out_h: u32,
) -> (u32, u32, Vec<u8>) {
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];
    if width == 0 || height == 0 || out_w == 0 || out_h == 0 {
        return (out_w, out_h, out);
    }
    for y in 0..out_h {
        for x in 0..out_w {
            let sx = x * width / out_w;
            let sy = y * height / out_h;
            let src = ((sy * width + sx) * 4) as usize;
            let dst = ((y * out_w + x) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    (out_w, out_h, out)
}

/// Centers the tray image on a transparent square so the Finder sizes do not
/// stretch it. The mark's pixels are copied unchanged.
pub fn square_rgba(width: u32, height: u32, rgba: &[u8]) -> (u32, Vec<u8>) {
    let size = width.max(height);
    let mut out = vec![0u8; (size * size * 4) as usize];
    let x0 = (size - width) / 2;
    let y0 = (size - height) / 2;
    for y in 0..height {
        for x in 0..width {
            let src = ((y * width + x) * 4) as usize;
            let dst = (((y + y0) * size + (x + x0)) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    (size, out)
}

pub fn write_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    writer
        .write_image_data(rgba)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    Ok(())
}

/// Same mark, with `pencil` (top-down alpha, one byte per output pixel) in place
/// of the drawn fallback. macOS passes the system pencil glyph here.
#[cfg(target_os = "macos")]
pub fn icon_with_pencil(pencil: &[u8]) -> (u32, u32, Vec<u8>) {
    icon_from_pencil(Some(pencil), [0, 0, 0])
}

/// AppKit draw rect (origin at the bottom left) for the system pencil, in the output image.
#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
fn map_x(src: f32) -> f32 {
    ICON_WIDTH as f32 / 2.0 + (src - SRC_CX) * MARK_SCALE
}

#[cfg(target_os = "macos")]
fn map_y(src: f32) -> f32 {
    ICON_HEIGHT as f32 / 2.0 + (src - SRC_CY) * MARK_SCALE
}

fn source_point(x: u32, y: u32) -> (f32, f32) {
    let sx = SRC_CX + (x as f32 + 0.5 - ICON_WIDTH as f32 / 2.0) / MARK_SCALE;
    let sy = SRC_CY + (y as f32 + 0.5 - ICON_HEIGHT as f32 / 2.0) / MARK_SCALE;
    (sx, sy)
}

#[cfg(target_os = "macos")]
fn icon_from_pencil(pencil: Option<&[u8]>, rgb: [u8; 3]) -> (u32, u32, Vec<u8>) {
    paint_icon(
        pencil.filter(|mask| mask.len() == (ICON_WIDTH * ICON_HEIGHT) as usize),
        rgb,
    )
}

#[cfg(not(target_os = "macos"))]
fn icon_from_pencil(rgb: [u8; 3]) -> (u32, u32, Vec<u8>) {
    paint_icon(None, rgb)
}

fn paint_icon(mask: Option<&[u8]>, rgb: [u8; 3]) -> (u32, u32, Vec<u8>) {
    let pixels = (ICON_WIDTH * ICON_HEIGHT) as usize;
    let mut data = vec![0u8; pixels * 4];
    for y in 0..ICON_HEIGHT {
        for x in 0..ICON_WIDTH {
            let (px, py) = source_point(x, y);
            let index = (y * ICON_WIDTH + x) as usize;
            let drawn = mask
                .map(|mask| mask[index] as f32 / 255.0)
                .unwrap_or_else(|| pencil_coverage(px, py));
            let glyph = drawn.max(person_coverage(px, py));
            let alpha = (glyph.clamp(0.0, 1.0) * 255.0).round() as u8;
            let i = index * 4;
            data[i] = rgb[0];
            data[i + 1] = rgb[1];
            data[i + 2] = rgb[2];
            data[i + 3] = alpha;
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

fn spawn_server(port: u16, address: Arc<Mutex<String>>) -> io::Result<Child> {
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
        .env("RUST_REG_HTTP", format!("0.0.0.0:{port}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let log = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let address = Arc::clone(&address);
        let log = Arc::clone(&log);
        thread::spawn(move || watch_address(stderr, address, log));
    }
    let started = address.lock().expect("address").clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if address.lock().expect("address").as_str() != started {
            let shown = address.lock().expect("address").clone();
            if shown.ends_with(&format!(":{port}")) {
                return Ok(child);
            }
            stop_child(child);
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("server listened on {shown}"),
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                thread::sleep(Duration::from_millis(100));
                return Err(startup_error(&log, Some(status), port));
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                stop_child(child);
                thread::sleep(Duration::from_millis(100));
                return Err(startup_error(&log, None, port));
            }
            Err(err) => return Err(err),
        }
    }
}

fn startup_error(log: &Mutex<String>, status: Option<ExitStatus>, port: u16) -> io::Error {
    let mut text = log.lock().expect("log").trim().to_string();
    if text.len() > 1200 {
        text = text[text.len() - 1200..].to_string();
        if let Some(index) = text.find('\n') {
            text = text[index + 1..].to_string();
        }
    }
    let message = if !text.is_empty() {
        text
    } else if let Some(status) = status {
        format!("server exited ({status})")
    } else {
        format!("server did not start listening on port {port}")
    };
    io::Error::new(io::ErrorKind::Other, message)
}

fn watch_address(stderr: impl std::io::Read, address: Arc<Mutex<String>>, log: Arc<Mutex<String>>) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        {
            let mut stored = log.lock().expect("log");
            if stored.len() > 8000 {
                stored.clear();
            }
            stored.push_str(&line);
            stored.push('\n');
        }
        if let Some(shown) = desk_address(&line) {
            *address.lock().expect("address") = shown;
        }
    }
}

/// Never block indefinitely. `Child::wait` / `Drop` can hang when signals do
/// not reap the process; abandon the handle instead of freezing Port / Quit.
fn stop_child(mut child: Child) {
    let grace = Duration::from_secs(2);
    #[cfg(unix)]
    {
        let pid = child.id();
        desk_log(format!("stop_child TERM pid={pid}"));
        if pid > 0 {
            let group = format!("-{pid}");
            let _ = Command::new("kill")
                .args(["-TERM", &group])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if wait_child_for(&mut child, grace) {
            desk_log(format!("stop_child reaped after TERM pid={pid}"));
            return;
        }
        desk_log(format!("stop_child KILL pid={pid}"));
        if pid > 0 {
            let group = format!("-{pid}");
            let _ = Command::new("kill")
                .args(["-KILL", &group])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = child.kill();
        if wait_child_for(&mut child, Duration::from_secs(1)) {
            desk_log(format!("stop_child reaped after KILL pid={pid}"));
            return;
        }
        desk_log(format!(
            "stop_child abandon pid={pid} (wait would block)"
        ));
        std::mem::forget(child);
        return;
    }
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if wait_child_for(&mut child, grace.saturating_add(Duration::from_secs(1))) {
            return;
        }
        std::mem::forget(child);
        return;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = child.kill();
        let _ = wait_child_for(&mut child, grace);
    }
}

fn wait_child_for(child: &mut Child, limit: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < limit {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return true,
        }
    }
    matches!(child.try_wait(), Ok(Some(_)) | Err(_))
}

/// Stop `old_child` / `old_port`, then start listening on `new_port`.
/// Safe to run on a worker thread (Linux Port apply).
#[cfg(target_os = "linux")]
pub fn restart_switch(
    old_child: Option<Child>,
    old_port: u16,
    new_port: u16,
    address: Arc<Mutex<String>>,
) -> (Option<Child>, String, Option<String>) {
    desk_log(format!(
        "restart_switch worker {old_port} → {new_port} old_open={} new_open={}",
        port_open(old_port),
        port_open(new_port)
    ));
    if let Some(child) = old_child {
        desk_log(format!("restart_switch stop old child pid={}", child.id()));
        stop_child(child);
    } else {
        desk_log("restart_switch no old child");
    }
    desk_log(format!(
        "restart_switch stop_port {old_port} (open={})",
        port_open(old_port)
    ));
    stop_port(old_port);
    desk_log(format!(
        "restart_switch after stop_port {old_port} open={}",
        port_open(old_port)
    ));
    if port_open(new_port) {
        let shown = format!("localhost:{new_port}");
        desk_log(format!("restart_switch adopt existing {shown}"));
        *address.lock().expect("address") = shown.clone();
        return (None, shown, None);
    }
    match spawn_server(new_port, Arc::clone(&address)) {
        Ok(child) => {
            let shown = address.lock().expect("address").clone();
            let shown = if shown.is_empty() {
                format!("localhost:{new_port}")
            } else {
                shown
            };
            desk_log(format!(
                "restart_switch spawned pid={} address={shown}",
                child.id()
            ));
            (Some(child), shown, None)
        }
        Err(err) => {
            let message = err.to_string();
            let shown = format!("failed on port {new_port}");
            desk_log(format!("restart_switch spawn fail: {message}"));
            *address.lock().expect("address") = shown.clone();
            (None, shown, Some(message))
        }
    }
}

/// PIDs listening on TCP `port`. Every external probe is bounded with
/// `timeout` — bare `lsof`/`fuser` have hung the Port-switch worker (and Quit)
/// on the Ubuntu UTM guest.
#[cfg(unix)]
fn listen_pids(port: u16) -> Vec<String> {
    let port_text = port.to_string();
    let ss = timed_output(
        "2",
        "ss",
        &["-ltnp", &format!("sport = :{port_text}")],
    );
    if let Some(output) = ss {
        let mut pids = Vec::new();
        for cap in output.split("pid=").skip(1) {
            let pid: String = cap
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !pid.is_empty() && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        if !pids.is_empty() {
            return pids;
        }
    }
    let lsof = timed_output(
        "2",
        "lsof",
        &["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"],
    );
    if let Some(output) = lsof {
        let pids: Vec<String> = output
            .lines()
            .map(str::trim)
            .filter(|pid| !pid.is_empty())
            .map(str::to_string)
            .collect();
        if !pids.is_empty() {
            return pids;
        }
    }
    // fuser prints PIDs on stdout and/or stderr depending on build.
    let fuser = timed_command_output("2", "fuser", &[&format!("{port}/tcp")]);
    if let Some((stdout, stderr)) = fuser {
        let mut pids = Vec::new();
        for text in [stdout.as_str(), stderr.as_str()] {
            for pid in text.split_whitespace() {
                if pid.chars().all(|c| c.is_ascii_digit()) && !pids.iter().any(|p| p == pid) {
                    pids.push(pid.to_string());
                }
            }
        }
        return pids;
    }
    Vec::new()
}

#[cfg(unix)]
fn timed_output(secs: &str, bin: &str, args: &[&str]) -> Option<String> {
    timed_command_output(secs, bin, args).map(|(stdout, _)| stdout)
}

#[cfg(unix)]
fn timed_command_output(secs: &str, bin: &str, args: &[&str]) -> Option<(String, String)> {
    let mut command = Command::new("timeout");
    command.arg(secs).arg(bin).args(args);
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()
        .or_else(|| {
            Command::new(bin)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .ok()
        })?;
    Some((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn stop_port(port: u16) {
    if !port_open(port) {
        desk_log(format!("stop_port {port} already free"));
        return;
    }
    #[cfg(unix)]
    {
        let pids = listen_pids(port);
        desk_log(format!("stop_port {port} listeners={pids:?}"));
    }
    let started = std::time::Instant::now();
    for round in 0..20 {
        if started.elapsed() > Duration::from_secs(5) {
            desk_log(format!(
                "stop_port {port} giving up after {:?}, still_open={}",
                started.elapsed(),
                port_open(port)
            ));
            return;
        }
        if !port_open(port) {
            desk_log(format!("stop_port {port} free after round {round}"));
            return;
        }
        #[cfg(unix)]
        {
            let pids = listen_pids(port);
            if pids.is_empty() {
                desk_log(format!(
                    "stop_port {port} open but no pids (round {round})"
                ));
            }
            for pid in &pids {
                let _ = Command::new("kill")
                    .args(["-TERM", pid])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            thread::sleep(Duration::from_millis(100));
            if port_open(port) {
                for pid in &pids {
                    let _ = Command::new("kill")
                        .args(["-KILL", pid])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
        #[cfg(windows)]
        {
            let _ = port;
        }
        thread::sleep(Duration::from_millis(50));
    }
    desk_log(format!(
        "stop_port {port} finished loops, still_open={}",
        port_open(port)
    ));
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

/// Lock path for stderr diagnostics (second-instance / already-running).
pub(crate) fn lock_path_for_log() -> PathBuf {
    lock_path()
}

/// Persistent desk log (stderr is often `/dev/null` after detach).
/// Primary: [`state_dir`]/`desk.log`. Mirror: `/tmp/rust-reg-desk.log`.
pub fn desk_log_path() -> PathBuf {
    state_dir().join("desk.log")
}

/// Append one line to stderr, [`desk_log_path`], and `/tmp/rust-reg-desk.log`.
pub fn desk_log(message: impl AsRef<str>) {
    let line = format!("rust-reg-desk: {}", message.as_ref());
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "{line}");
    let _ = err.flush();
    for path in [desk_log_path(), PathBuf::from("/tmp/rust-reg-desk.log")] {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

fn remembered_port() -> u16 {
    read_saved_port(&port_path())
        // Older builds stored the port next to the runtime lock (cleared on logout).
        .or_else(|| read_saved_port(&lock_path().with_file_name("rust-reg-desk.port")))
        .unwrap_or(DEFAULT_PORT)
}

fn read_saved_port(path: &Path) -> Option<u16> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .filter(|port: &u16| *port != 0)
}

fn remember_port(port: u16) {
    let path = port_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, port.to_string());
    desk_log(format!("remembered port {port} at {}", path.display()));
}

/// Durable last-used listen port. Not next to the lock: runtime dirs are wiped
/// on logout/reboot.
fn port_path() -> PathBuf {
    state_dir().join("desk.port")
}

/// Per-user state for desk preferences (port) and logs.
///
/// Override with `RUST_REG_DESK_STATE`. Otherwise:
/// Linux `XDG_STATE_HOME/rust-reg` (default `~/.local/state/rust-reg`),
/// macOS `~/Library/Application Support/rust-reg`,
/// Windows `%LOCALAPPDATA%\rust-reg`.
pub(crate) fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RUST_REG_DESK_STATE") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        return home.join("Library/Application Support/rust-reg");
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(base).join("rust-reg");
        }
        return std::env::temp_dir().join("rust-reg");
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let dir = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|dir| !dir.as_os_str().is_empty())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(std::env::temp_dir);
        dir.join("rust-reg")
    }
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

/// `Contents/prefix` inside a drag-installed app. The writable copy lives in
/// Application Support so replacing the app does not wipe the event files.
fn bundled_template(exe: &Path, run_name: &str) -> Option<PathBuf> {
    let contents = exe.parent()?.parent()?;
    if contents.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        return None;
    }
    let prefix = contents.join("prefix");
    if prefix.join("bin").join(run_name).is_file() {
        Some(prefix)
    } else {
        None
    }
}

fn support_prefix() -> PathBuf {
    if let Some(dir) = std::env::var_os("RUST_REG_DESK_PREFIX") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        return home.join("Library/Application Support/rust-reg");
    }
    #[cfg(not(target_os = "macos"))]
    std::env::temp_dir().join("rust-reg")
}

fn materialize(template: &Path, support: &Path) -> std::io::Result<PathBuf> {
    let (run_name, _) = binary_names(Path::new("reg-server"));
    if support.join("updates/live").is_file() && support.join("bin").join(run_name).is_file() {
        return Ok(support.to_path_buf());
    }
    copy_tree(template, support)?;
    Ok(support.to_path_buf())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
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
    fn square_icon_keeps_the_tray_pixels() {
        let rgba = [0, 0, 0, 255, 0, 0, 0, 0];
        let (size, square) = square_rgba(2, 1, &rgba);
        assert_eq!(size, 2);
        assert_eq!(&square[0..4], &[0, 0, 0, 255]);
        assert_eq!(&square[4..8], &[0, 0, 0, 0]);
        assert_eq!(&square[8..16], &[0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn linux_tray_mark_is_not_black_only_alpha() {
        let (_, _, rgba) = paint_icon(None, [0x1f, 0x6f, 0xeb]);
        let mut opaque = 0;
        for pixel in rgba.chunks_exact(4) {
            if pixel[3] > 20 {
                opaque += 1;
                assert_eq!(&pixel[0..3], &[0x1f, 0x6f, 0xeb]);
            }
        }
        assert!(opaque > 400, "opaque pixels {opaque}");
    }

    #[test]
    fn scale_rgba_keeps_channels() {
        let src = [0x1f, 0x6f, 0xeb, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (w, h, out) = scale_rgba(2, 2, &src, 1, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(&out[..], &[0x1f, 0x6f, 0xeb, 255]);
    }

    #[test]
    fn listen_url_opens_a_live_address() {
        assert_eq!(
            listen_url("localhost:80"),
            Some("http://localhost:80".to_string())
        );
        assert_eq!(listen_url("failed on port 8080"), None);
        assert_eq!(listen_url(""), None);
    }

    #[test]
    fn replacement_port_accepts_only_a_different_port() {
        assert_eq!(replacement_port(8080, "8080"), None);
        assert_eq!(replacement_port(8080, " 8080 "), None);
        assert_eq!(replacement_port(8080, ""), None);
        assert_eq!(replacement_port(8080, "0"), None);
        assert_eq!(replacement_port(8080, "nope"), None);
        assert_eq!(replacement_port(8080, "65536"), None);
        assert_eq!(replacement_port(8080, "8081"), Some(8081));
    }

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

    #[cfg(unix)]
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
    fn mac_app_copies_the_bundled_prefix() {
        let root = std::env::temp_dir().join(format!("reg-server-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let exe = root.join("reg-server.app/Contents/MacOS/reg-server");
        let (run_name, _) = binary_names(&exe);
        let template = root.join("reg-server.app/Contents/prefix");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(template.join("bin")).unwrap();
        std::fs::create_dir_all(template.join("updates")).unwrap();
        std::fs::write(&exe, b"tray").unwrap();
        std::fs::write(template.join("bin").join(run_name), b"run").unwrap();
        std::fs::write(template.join("updates/live"), b"2.0.0\n").unwrap();
        let support = root.join("support");
        let launch = launch_for_in(&exe, &support);
        assert_eq!(launch.program, support.join("bin").join(run_name));
        assert_eq!(
            launch.args,
            vec!["--prefix".to_string(), support.display().to_string()]
        );
        assert_eq!(
            std::fs::read(support.join("updates/live")).unwrap(),
            b"2.0.0\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_app_uses_the_library_prefix() {
        let support = std::env::temp_dir().join(format!("reg-server-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&support);
        let launch = launch_for_in(
            Path::new("/Applications/reg-server.app/Contents/MacOS/reg-server"),
            &support,
        );
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
    fn mac_app_uses_application_support_when_library_is_missing() {
        let root = std::env::temp_dir().join(format!("reg-server-support-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let support = root.join("support");
        let (run_name, _) = binary_names(Path::new("reg-server"));
        std::fs::create_dir_all(support.join("bin")).unwrap();
        std::fs::write(support.join("bin").join(run_name), b"run").unwrap();
        let launch = launch_for_in(
            Path::new("/Applications/reg-server.app/Contents/MacOS/reg-server"),
            &support,
        );
        assert_eq!(launch.program, support.join("bin").join(run_name));
        assert_eq!(
            launch.args,
            vec!["--prefix".to_string(), support.display().to_string()]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn development_copy_starts_the_sibling_server() {
        let desk = Path::new("/tmp/target/debug/rust-reg-desk");
        let (_, server_name) = binary_names(desk);
        let launch = launch_for(desk);
        assert_eq!(launch.program, desk.with_file_name(server_name));
        assert!(launch.args.is_empty());
    }

    #[test]
    fn read_saved_port_rejects_zero_and_garbage() {
        let dir = std::env::temp_dir().join(format!(
            "rust-reg-port-parse-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desk.port");
        std::fs::write(&path, "8081\n").unwrap();
        assert_eq!(read_saved_port(&path), Some(8081));
        std::fs::write(&path, "0").unwrap();
        assert_eq!(read_saved_port(&path), None);
        std::fs::write(&path, "nope").unwrap();
        assert_eq!(read_saved_port(&path), None);
        assert_eq!(read_saved_port(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remember_port_roundtrips_through_state_dir() {
        static GUARD: Mutex<()> = Mutex::new(());
        let _lock = GUARD.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "rust-reg-desk-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("RUST_REG_DESK_STATE", &dir);
        remember_port(9091);
        assert_eq!(
            std::fs::read_to_string(dir.join("desk.port")).unwrap().trim(),
            "9091"
        );
        assert_eq!(remembered_port(), 9091);
        std::env::remove_var("RUST_REG_DESK_STATE");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
