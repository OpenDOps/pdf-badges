//! Linux desk: a StatusNotifierItem. The deb launches this binary from
//! `/usr/share/applications/rust-reg.desktop`.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray};

use crate::server::{self, Opened};

struct Desk {
    address: Arc<Mutex<String>>,
    quit: mpsc::Sender<()>,
}

impl Tray for Desk {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "reg-server".into()
    }

    fn title(&self) -> String {
        "reg-server".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let (width, height, rgba) = server::icon_rgba();
        vec![Icon {
            width: width as i32,
            height: height as i32,
            data: argb32(&rgba),
        }]
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let label = self.address.lock().expect("address").clone();
        let quit = self.quit.clone();
        vec![
            StandardItem {
                label,
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(move |_| {
                    let _ = quit.send(());
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = match server::Session::open()? {
        Opened::AlreadyRunning => return Ok(()),
        Opened::Ready(session) => session,
    };
    let (tx, rx) = mpsc::channel();
    let handle = Desk {
        address: Arc::clone(session.address()),
        quit: tx,
    }
    .spawn()?;
    let _ = rx.recv();
    session.stop();
    handle.shutdown().wait();
    Ok(())
}

pub(crate) fn stop_installed_service() {
    use std::process::{Command, Stdio};
    let active = Command::new("systemctl")
        .args(["is-active", "--quiet", "rust-reg.service"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !active {
        return;
    }
    let stopped = Command::new("systemctl")
        .args(["stop", "rust-reg.service"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !stopped {
        let _ = Command::new("pkexec")
            .args(["systemctl", "stop", "rust-reg.service"])
            .status();
    }
}

fn argb32(rgba: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        data.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
    }
    data
}
