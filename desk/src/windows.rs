//! Windows desk: a notification-area icon. The installer adds a Start Menu
//! shortcut named reg-server that launches this binary.

use std::mem::zeroed;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MessageBoxW, PeekMessageW, TranslateMessage, MB_ICONERROR,
    MB_ICONINFORMATION, MB_OK, MSG, PM_REMOVE,
};

use crate::server::{self, Opened};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = match server::Session::open()? {
        Opened::AlreadyRunning => {
            eprintln!(
                "rust-reg-desk: already running ({})",
                server::lock_path_for_log().display()
            );
            return Ok(());
        }
        Opened::Ready(session) => session,
    };
    let probe = server::upgrade_probe_letter();
    let (width, height, rgba) = server::icon_rgba();
    let icon = tray_icon::Icon::from_rgba(rgba, width, height)?;
    let mut shown = session.address_text();
    let address = MenuItem::with_id(
        "address",
        &shown,
        server::listen_url(&shown).is_some(),
        None,
    );
    let port_item = MenuItem::with_id("port", "Port…", true, None);
    let upgrade_probe = server::show_upgrade_probe_menu(probe).then(|| {
        MenuItem::with_id("upgrade-probe", server::UPGRADE_PROBE_MENU, true, None)
    });
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu = Menu::new();
    menu.append(&address)?;
    menu.append(&port_item)?;
    if let Some(item) = upgrade_probe.as_ref() {
        menu.append(item)?;
    }
    menu.append(&quit)?;
    let tooltip = match probe {
        Some(letter) => format!("reg-server (probe {letter})"),
        None => "reg-server".into(),
    };
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .with_tooltip(&tooltip)
        .with_icon(icon)
        .build()?;

    if let Some(message) = session.failure() {
        show_error(&message);
    }

    let address_id = address.id().clone();
    let port_id = port_item.id().clone();
    let upgrade_probe_id = upgrade_probe.as_ref().map(|item| item.id().clone());
    let quit_id = quit.id().clone();
    loop {
        pump_messages();
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == quit_id {
                session.stop();
                break;
            }
            if event.id == address_id {
                server::open_listen(&session.address_text());
            }
            if event.id == port_id {
                if let Some(port) = ask_port(session.port()) {
                    session.restart(port);
                    shown = session.address_text();
                    set_address_item(&address, &shown);
                    if let Some(message) = session.failure() {
                        show_error(&message);
                    }
                }
            }
            if upgrade_probe_id.as_ref() == Some(&event.id) {
                server::desk_log("action UpgradeProbe");
                show_info("Tray marker B is active.");
            }
        }
        let next = session.address_text();
        if next != shown {
            set_address_item(&address, &next);
            shown = next;
        }
        thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

fn set_address_item(item: &MenuItem, text: &str) {
    item.set_text(text);
    item.set_enabled(server::listen_url(text).is_some());
}

pub(crate) fn stop_installed_service() {
    let _ = Command::new("sc.exe")
        .args(["stop", "rust-reg"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn ask_port(current: u16) -> Option<u16> {
    let script = format!(
        "Add-Type -AssemblyName Microsoft.VisualBasic; \
         $value = [Microsoft.VisualBasic.Interaction]::InputBox('Port','reg-server','{current}'); \
         if ($value -ne '') {{ Write-Output $value }}"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-Command", &script])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    server::replacement_port(current, text.trim())
}

fn show_error(message: &str) {
    show_message(message, "reg-server", MB_OK | MB_ICONERROR);
}

fn show_info(message: &str) {
    show_message(message, server::UPGRADE_PROBE_MENU, MB_OK | MB_ICONINFORMATION);
}

fn show_message(message: &str, title: &str, style: u32) {
    use std::os::windows::ffi::OsStrExt;
    let mut text: Vec<u16> = std::ffi::OsStr::new(message).encode_wide().collect();
    text.push(0);
    let mut title: Vec<u16> = std::ffi::OsStr::new(title).encode_wide().collect();
    title.push(0);
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            style,
        );
    }
}

fn pump_messages() {
    unsafe {
        let mut message: MSG = zeroed();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
