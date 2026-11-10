//! Linux desk: a StatusNotifierItem. The deb launches this binary from
//! `/usr/share/applications/rust-reg.desktop`.

use std::io::{self, Read, Write};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use std::path::Path;

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray};

use crate::server::{self, Opened};

enum Action {
    Quit,
    AskPort,
    Open,
    UpgradeProbe,
    ShowError(String),
    /// Worker finished [`server::restart_switch`].
    PortSwitched {
        port: u16,
        child: Option<Child>,
        address: String,
        failure: Option<String>,
    },
}

/// Dialog child started from the main loop (not from menu `activate`).
enum Pending {
    Port { child: Child, current: u16 },
    Error { child: Child },
    /// Port apply is stopping/starting the server on a worker thread.
    Restarting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogTool {
    Zenity,
    KDialog,
    Yad,
}

struct Desk {
    address: Arc<Mutex<String>>,
    actions: mpsc::Sender<Action>,
}

impl Tray for Desk {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "reg-server".into()
    }

    fn title(&self) -> String {
        match server::upgrade_probe_letter() {
            Some(letter) => format!("reg-server (probe {letter})"),
            None => "reg-server".into(),
        }
    }

    /// Theme name only. Absolute file paths and IconPixmap have both crashed
    /// gnome-shell / AppIndicator on this Ubuntu host (including on Quit).
    fn icon_name(&self) -> String {
        tray_icon_name()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        Vec::new()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let label = self.address.lock().expect("address").clone();
        let quit = self.actions.clone();
        let ask = self.actions.clone();
        let open = self.actions.clone();
        let mut items = vec![
            StandardItem {
                label: label.clone(),
                enabled: server::listen_url(&label).is_some(),
                activate: Box::new(move |_| {
                    let _ = open.send(Action::Open);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Port…".into(),
                activate: Box::new(move |_| {
                    let _ = ask.send(Action::AskPort);
                }),
                ..Default::default()
            }
            .into(),
        ];
        if server::show_upgrade_probe_menu(server::upgrade_probe_letter()) {
            let probe = self.actions.clone();
            items.push(
                StandardItem {
                    label: server::UPGRADE_PROBE_MENU.into(),
                    activate: Box::new(move |_| {
                        let _ = probe.send(Action::UpgradeProbe);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(move |_| {
                    let _ = quit.send(Action::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = match server::Session::open()? {
        Opened::AlreadyRunning => {
            server::desk_log(format!(
                "already running ({})",
                server::lock_path_for_log().display()
            ));
            // From Applications, a silent instant exit looks like a crash.
            if std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE").is_some()
                || std::env::var_os("DESKTOP_STARTUP_ID").is_some()
                || std::env::var_os("RUST_REG_DESK_NO_DETACH").is_some()
            {
                let _ = Command::new("zenity")
                    .args([
                        "--info",
                        "--title",
                        "reg-server",
                        "--text",
                        "reg-server is already running in the tray.",
                    ])
                    .env("GDK_BACKEND", "x11")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            return Ok(());
        }
        Opened::Ready(session) => session,
    };
    server::desk_log(format!("log file {}", server::desk_log_path().display()));
    let startup_error = session.failure();
    if let Some(message) = startup_error.as_ref() {
        server::desk_log(message.clone());
    }
    let (tx, rx) = mpsc::channel();
    let handle = Desk {
        address: Arc::clone(session.address()),
        actions: tx.clone(),
    }
    .spawn()
    .map_err(|err| {
        server::desk_log(format!("tray spawn failed: {err}"));
        err
    })?;
    server::desk_log(format!(
        "tray up address={} port={} icon={}",
        session.address_text(),
        session.port(),
        tray_icon_name()
    ));

    let mut pending = None;
    if let Some(message) = startup_error {
        pending = spawn_error_dialog(&message);
    }

    loop {
        let action = if pending.is_none() {
            match rx.recv() {
                Ok(action) => Some(action),
                Err(_) => break,
            }
        } else {
            match rx.try_recv() {
                Ok(action) => Some(action),
                Err(TryRecvError::Empty) => {
                    if let Some(finished) = poll_pending(&mut pending)? {
                        apply_finished(&mut session, finished, &tx, &mut pending);
                        if matches!(pending, Some(Pending::Restarting)) {
                            let _ = handle.update(|_| {});
                        }
                        None
                    } else {
                        match rx.recv_timeout(Duration::from_millis(50)) {
                            Ok(action) => Some(action),
                            Err(RecvTimeoutError::Timeout) => None,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => break,
            }
        };

        match action {
            Some(Action::Quit) => {
                server::desk_log(format!(
                    "action Quit pending={} address={}",
                    pending_label(pending.as_ref()),
                    session.address_text()
                ));
                // Drop the dialog without apply_finished: Quit must not restart.
                if let Some(open) = pending.take() {
                    terminate_pending(open);
                }
                break;
            }
            Some(Action::Open) => {
                server::desk_log(format!("action Open {}", session.address_text()));
                server::open_listen(&session.address_text());
            }
            Some(Action::UpgradeProbe) => {
                server::desk_log("action UpgradeProbe");
                if pending.is_none() {
                    pending = spawn_info_dialog("Tray marker B is active.");
                }
            }
            Some(Action::AskPort) => {
                server::desk_log(format!(
                    "action AskPort pending={} port={}",
                    pending_label(pending.as_ref()),
                    session.port()
                ));
                if matches!(pending, Some(Pending::Restarting)) {
                    server::desk_log("AskPort ignored (restart in flight)");
                } else if let Some(Pending::Port { child, .. }) = pending.as_ref() {
                    raise_dialog(child.id(), "Port");
                } else {
                    // A failed Port apply may have left an error dialog up;
                    // Port… used to be a silent no-op until that dialog closed.
                    if let Some(open) = pending.take() {
                        terminate_pending(open);
                    }
                    pending = spawn_port_dialog(session.port());
                    server::desk_log(format!(
                        "Port dialog spawned={}",
                        pending.is_some()
                    ));
                    if let Some(Pending::Port { child, .. }) = pending.as_ref() {
                        // Window appears a beat after spawn; raise it over others.
                        raise_dialog_soon(child.id(), "Port");
                    }
                }
            }
            Some(Action::ShowError(message)) => {
                server::desk_log(format!(
                    "action ShowError pending={} msg={message}",
                    pending_label(pending.as_ref())
                ));
                if matches!(pending, Some(Pending::Port { .. } | Pending::Restarting)) {
                    // Keep Port dialog / switch; surface the failure after.
                } else {
                    if let Some(open) = pending.take() {
                        terminate_pending(open);
                    }
                    pending = spawn_error_dialog(&message);
                }
            }
            Some(Action::PortSwitched {
                port,
                child,
                address,
                failure,
            }) => {
                server::desk_log(format!(
                    "action PortSwitched port={port} address={address} failure={failure:?}"
                ));
                if matches!(pending, Some(Pending::Restarting)) {
                    pending = None;
                }
                session.finish_port_switch(port, child, address, failure.clone());
                // Force AppIndicator to rebuild the menu label.
                let _ = handle.update(|_| {});
                if let Some(message) = failure {
                    if pending.is_none() {
                        pending = spawn_error_dialog(&message);
                    }
                }
            }
            None => {}
        }
    }

    // Stop the server first. Skip ksni's graceful unregister: AppIndicator +
    // gnome-shell has crashed the whole session on StatusNotifier removal here
    // (looks like an instant logout). Forget the handle so Drop does not send
    // shutdown either; process exit tears down the D-Bus connection.
    // Do not call handle.shutdown(): AppIndicator/gnome-shell has crashed the
    // session on StatusNotifier unregister. Forget the handle and leave the
    // process; a normal Ok return is a clean exit for GNOME (not a "crash").
    std::mem::forget(handle);
    server::desk_log("quit: stopping session (timed)");
    let stopper = thread::spawn(move || {
        session.stop();
        server::desk_log("quit: stop returned");
    });
    let mut finished = false;
    for _ in 0..60 {
        if stopper.is_finished() {
            finished = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    if finished {
        let _ = stopper.join();
    } else {
        server::desk_log("quit: stop still running after 3s — leaving process");
        std::mem::forget(stopper);
    }
    server::desk_log("quit: done");
    Ok(())
}

fn pending_label(pending: Option<&Pending>) -> &'static str {
    match pending {
        None => "none",
        Some(Pending::Port { .. }) => "port-dialog",
        Some(Pending::Error { .. }) => "error-dialog",
        Some(Pending::Restarting) => "restarting",
    }
}

enum Finished {
    Port {
        current: u16,
        output: Option<Output>,
    },
    Error,
}

fn poll_pending(pending: &mut Option<Pending>) -> io::Result<Option<Finished>> {
    let exited = match pending.as_mut() {
        Some(Pending::Port { child, .. } | Pending::Error { child }) => child.try_wait()?,
        Some(Pending::Restarting) | None => return Ok(None),
    };
    let Some(status) = exited else {
        return Ok(None);
    };
    Ok(Some(match pending.take().expect("pending") {
        Pending::Port { mut child, current } => {
            let mut stdout = Vec::new();
            if let Some(mut out) = child.stdout.take() {
                let _ = out.read_to_end(&mut stdout);
            }
            let output = status.success().then_some(Output {
                status,
                stdout,
                stderr: Vec::new(),
            });
            Finished::Port { current, output }
        }
        Pending::Error { mut child } => {
            // try_wait already reaped; clear any pipe and ignore.
            let _ = child.stdout.take();
            Finished::Error
        }
        Pending::Restarting => unreachable!("Restarting has no child to poll"),
    }))
}

fn apply_finished(
    session: &mut server::Session,
    finished: Finished,
    actions: &mpsc::Sender<Action>,
    pending: &mut Option<Pending>,
) {
    match finished {
        Finished::Error => {}
        Finished::Port { current, output } => {
            let Some(output) = output else {
                return;
            };
            let text = String::from_utf8_lossy(&output.stdout);
            let raw = text.trim();
            let Some(next) = server::replacement_port(current, raw) else {
                server::desk_log(format!(
                    "Port dialog dismissed or unchanged current={current} raw={raw:?}"
                ));
                return;
            };
            server::desk_log(format!("Port apply {current} → {next}"));
            // Stop/start on a worker so the tray loop keeps handling Quit / Port….
            let (old_child, old_port) = session.prepare_port_switch(next);
            let address = Arc::clone(session.address());
            let tx = actions.clone();
            *pending = Some(Pending::Restarting);
            thread::spawn(move || {
                server::desk_log(format!("port switch worker start {old_port} → {next}"));
                let (child, shown, failure) =
                    server::restart_switch(old_child, old_port, next, address);
                server::desk_log(format!(
                    "port switch worker done → {shown} failure={failure:?}"
                ));
                if let Err(err) = tx.send(Action::PortSwitched {
                    port: next,
                    child,
                    address: shown,
                    failure,
                }) {
                    server::desk_log(format!("port switch send PortSwitched failed: {err}"));
                }
            });
        }
    }
}

/// SIGTERM the dialog, then SIGKILL if it stays up. Caller must not treat the
/// exit as **Apply and restart**.
fn terminate_pending(pending: Pending) {
    let mut child = match pending {
        Pending::Port { child, .. } | Pending::Error { child } => child,
        Pending::Restarting => return,
    };
    let pid = child.id() as i32;
    server::desk_log(format!("terminate_pending pid={pid}"));
    if pid > 0 {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return,
        }
    }
    let _ = child.kill();
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return,
        }
    }
    server::desk_log(format!("terminate_pending abandon pid={pid}"));
    std::mem::forget(child);
}

/// Raise after the toolkit has mapped the window (spawn is not instant).
fn raise_dialog_soon(pid: u32, title: &'static str) {
    thread::spawn(move || {
        for delay_ms in [50, 100, 200, 400] {
            thread::sleep(Duration::from_millis(delay_ms));
            if raise_dialog(pid, title) {
                server::desk_log(format!(
                    "raised dialog pid={pid} title={title} after {delay_ms}ms"
                ));
                return;
            }
        }
        server::desk_log(format!(
            "raise dialog failed pid={pid} title={title}"
        ));
    });
}

/// Bring a dialog to the front (above other windows) and focus it.
/// Returns true when a raise tool reported success.
fn raise_dialog(pid: u32, title: &str) -> bool {
    if pid == 0 {
        return false;
    }
    let pid_text = pid.to_string();
    let mut ok = false;
    if command_exists("xdotool") {
        // windowraise + activate; --sync waits until the window exists.
        let raised = Command::new("xdotool")
            .args([
                "search",
                "--sync",
                "--onlyvisible",
                "--pid",
                &pid_text,
                "windowraise",
                "windowactivate",
                "--sync",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        ok |= raised;
        if !raised {
            // Fallback without --onlyvisible (some WMs hide the hint briefly).
            ok |= Command::new("xdotool")
                .args([
                    "search",
                    "--pid",
                    &pid_text,
                    "windowraise",
                    "windowfocus",
                    "windowactivate",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
        }
    }
    if command_exists("wmctrl") {
        // Keep above other windows while activating (GNOME focus-stealing).
        let _ = Command::new("wmctrl")
            .args(["-F", "-r", title, "-b", "add,above"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        ok |= Command::new("wmctrl")
            .args(["-F", "-a", title])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        ok |= Command::new("wmctrl")
            .args(["-a", title])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
    }
    ok
}

fn dialog_tool() -> Option<DialogTool> {
    static TOOL: OnceLock<Option<DialogTool>> = OnceLock::new();
    *TOOL.get_or_init(detect_dialog_tool)
}

fn detect_dialog_tool() -> Option<DialogTool> {
    for (tool, bin) in [
        (DialogTool::Zenity, "zenity"),
        (DialogTool::KDialog, "kdialog"),
        (DialogTool::Yad, "yad"),
    ] {
        if command_exists(bin) {
            return Some(tool);
        }
    }
    None
}

fn command_exists(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn tray_icon_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        // Theme name only (not a filesystem path — those crash AppIndicator on
        // unregister). PNGs are the macOS mark shipped by the deb.
        if Path::new("/usr/share/icons/hicolor/48x48/apps/rust-reg.png").is_file()
            || Path::new("/usr/share/pixmaps/rust-reg.png").is_file()
            || Path::new("/opt/rust-reg/share/icons/rust-reg.png").is_file()
        {
            return "rust-reg".into();
        }
        "network-server".into()
    })
    .clone()
}

fn missing_dialog_hint() {
    let _ = writeln!(
        io::stderr(),
        "reg-server: no dialog tool (install zenity, kdialog, or yad)"
    );
}

fn spawn_error_dialog(message: &str) -> Option<Pending> {
    spawn_message_dialog(message, true)
}

fn spawn_info_dialog(message: &str) -> Option<Pending> {
    spawn_message_dialog(message, false)
}

fn spawn_message_dialog(message: &str, error: bool) -> Option<Pending> {
    let Some(tool) = dialog_tool() else {
        missing_dialog_hint();
        let _ = writeln!(io::stderr(), "reg-server: {message}");
        return None;
    };
    let title = if error {
        "reg-server"
    } else {
        server::UPGRADE_PROBE_MENU
    };
    let child = match (tool, error) {
        (DialogTool::Zenity, true) => Command::new("zenity")
            .args(["--error", "--title", title, "--text", message])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        (DialogTool::Zenity, false) => Command::new("zenity")
            .args(["--info", "--title", title, "--text", message])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        (DialogTool::KDialog, true) => Command::new("kdialog")
            .args(["--error", message, "--title", title])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        (DialogTool::KDialog, false) => Command::new("kdialog")
            .args(["--msgbox", message, "--title", title])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        (DialogTool::Yad, true) => Command::new("yad")
            .args(["--error", "--title", title, "--text", message])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        (DialogTool::Yad, false) => Command::new("yad")
            .args(["--info", "--title", title, "--text", message])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
    };
    match child {
        Ok(child) => Some(Pending::Error { child }),
        Err(err) => {
            let _ = writeln!(io::stderr(), "reg-server: dialog failed: {err}");
            let _ = writeln!(io::stderr(), "reg-server: {message}");
            None
        }
    }
}

/// Prefer X11 so xdotool/wmctrl can raise the dialog under Ubuntu Wayland.
fn dialog_command(bin: &str) -> Command {
    let mut command = Command::new(bin);
    command.env("GDK_BACKEND", "x11");
    command
}

fn spawn_port_dialog(current: u16) -> Option<Pending> {
    let Some(tool) = dialog_tool() else {
        missing_dialog_hint();
        return None;
    };
    let current_text = current.to_string();
    let child = match tool {
        DialogTool::Zenity => dialog_command("zenity")
            .args([
                "--entry",
                "--modal",
                "--title",
                "Port",
                "--text",
                "Port",
                "--entry-text",
                &current_text,
                "--ok-label",
                "Apply and restart",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn(),
        DialogTool::KDialog => dialog_command("kdialog")
            .args([
                "--inputbox",
                "Port",
                &current_text,
                "--title",
                "Port",
                "--ok-label",
                "Apply and restart",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn(),
        DialogTool::Yad => dialog_command("yad")
            .args([
                "--entry",
                "--on-top",
                "--title",
                "Port",
                "--text",
                "Port",
                "--entry-text",
                &current_text,
                "--button",
                "Apply and restart:0",
                "--button",
                "Cancel:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn(),
    };
    match child {
        Ok(child) => Some(Pending::Port { child, current }),
        Err(err) => {
            let _ = writeln!(io::stderr(), "reg-server: dialog failed: {err}");
            None
        }
    }
}

