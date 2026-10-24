//! macOS desk: a menu-bar status item. The pkg installs
//! `/Applications/reg-server.app`, which launches this binary.

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBitmapImageRep, NSCalibratedRGBColorSpace,
    NSEventMask, NSGraphicsContext, NSImage, NSImageSymbolConfiguration,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;

use crate::server::{self, Opened};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(marker) = MainThreadMarker::new() else {
        return Err("reg-server desk must run on the main thread".into());
    };
    let app = NSApplication::sharedApplication(marker);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let mut session = match server::Session::open()? {
        Opened::AlreadyRunning => return Ok(()),
        Opened::Ready(session) => session,
    };
    let (width, height, rgba) = menu_icon();
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
        .with_icon_templated(icon)
        .build()?;

    let quit_id = quit.id().clone();
    let mut shown = session.address_text();
    loop {
        while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(NSDate::distantPast().as_ref()),
            unsafe { NSDefaultRunLoopMode },
            true,
        ) {
            app.sendEvent(&event);
        }
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

fn menu_icon() -> (u32, u32, Vec<u8>) {
    match system_pencil_alpha() {
        Some(alpha) => server::icon_with_pencil(&alpha),
        None => server::icon_rgba(),
    }
}

/// Alpha mask of the macOS SF Symbol `pencil`, drawn on the left of the mark.
/// The menu bar paints this with the person as one template image.
fn system_pencil_alpha() -> Option<Vec<u8>> {
    let width = server::ICON_WIDTH as usize;
    let height = server::ICON_HEIGHT as usize;
    let (x, y, w, h) = server::pencil_symbol_rect();
    let name = NSString::from_str("pencil");
    let symbol = NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, None)?;
    let symbol = symbol
        .imageWithSymbolConfiguration(
            &NSImageSymbolConfiguration::configurationPreferringMonochrome(),
        )
        .unwrap_or(symbol);
    symbol.setTemplate(false);
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width as isize,
            height as isize,
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            (width * 4) as isize,
            32,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    symbol.drawInRect(NSRect {
        origin: NSPoint { x, y },
        size: NSSize {
            width: w,
            height: h,
        },
    });
    NSGraphicsContext::restoreGraphicsState_class();

    let ptr = rep.bitmapData();
    if ptr.is_null() {
        return None;
    }
    let row_bytes = rep.bytesPerRow() as usize;
    let mut alpha = vec![0u8; width * height];
    for y in 0..height {
        let row = unsafe { std::slice::from_raw_parts(ptr.add(y * row_bytes), width * 4) };
        for x in 0..width {
            let pixel = &row[x * 4..x * 4 + 4];
            alpha[y * width + x] = pixel.iter().copied().max().unwrap_or(0);
        }
    }
    let ink = alpha.iter().filter(|value| **value > 20).count();
    if ink < 100 {
        return None;
    }
    Some(alpha)
}

#[cfg(test)]
mod tests {
    use super::menu_icon;
    use crate::server;

    #[test]
    fn system_pencil_sits_beside_the_person() {
        let (width, height, rgba) = menu_icon();
        assert_eq!((width, height), (server::ICON_WIDTH, server::ICON_HEIGHT));
        let opaque = |x0, x1| {
            let mut count = 0;
            for y in 0..height {
                for x in x0..x1 {
                    if rgba[((y * width + x) * 4 + 3) as usize] > 20 {
                        count += 1;
                    }
                }
            }
            count
        };
        let pencil = opaque(0, 64);
        let person = opaque(70, 128);
        assert!(pencil > 400, "pencil pixels {pencil}");
        assert!(person > 400, "person pixels {person}");
    }
}

pub(crate) fn stop_installed_service() {
    let loaded = Command::new("launchctl")
        .args(["print", "system/com.opendops.rust-reg"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !loaded {
        return;
    }
    let stopped = Command::new("launchctl")
        .args(["bootout", "system/com.opendops.rust-reg"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !stopped {
        let _ = Command::new("osascript")
            .args([
                "-e",
                "do shell script \"launchctl bootout system/com.opendops.rust-reg\" with administrator privileges",
            ])
            .status();
    }
}
