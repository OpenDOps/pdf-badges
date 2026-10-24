//! Windows desk: a notification-area icon. The installer adds a Start Menu
//! shortcut named reg-server that launches this binary.

use std::mem::zeroed;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

use crate::server::{self, Opened};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = match server::Session::open()? {
        Opened::AlreadyRunning => return Ok(()),
        Opened::Ready(session) => session,
    };
    let (width, height, rgba) = server::icon_rgba();
    let icon = tray_icon::Icon::from_rgba(rgba, width, height)?;
    let address = MenuItem::with_id("address", session.address_text(), false, None);
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu = Menu::new();
    menu.append(&address)?;
    menu.append(&quit)?;
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .with_tooltip("reg-server")
        .with_icon(icon)
        .build()?;

    let quit_id = quit.id().clone();
    let mut shown = session.address_text();
    loop {
        pump_messages();
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == quit_id {
                session.stop();
                break;
            }
        }
        let next = session.address_text();
        if next != shown {
            address.set_text(&next);
            shown = next;
        }
        thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

pub(crate) fn stop_installed_service() {
    let _ = Command::new("sc.exe")
        .args(["stop", "rust-reg"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
