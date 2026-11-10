//! macOS desk: a menu-bar status item. The pkg installs
//! `/Applications/reg-server.app`, which launches this binary.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSAlert, NSAlertStyle, NSApplication, NSApplicationActivationOptions,
    NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageRep, NSButton,
    NSCalibratedRGBColorSpace, NSControlTextEditingDelegate, NSEventMask, NSGraphicsContext,
    NSImage, NSImageSymbolConfiguration, NSModalPanelWindowLevel, NSPanel, NSRunningApplication,
    NSTextField, NSTextFieldDelegate, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSNotification, NSObject, NSPoint, NSRect, NSSize, NSString,
};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;

use crate::server::{self, Opened};

/// Button result from the non-modal port panel. Quit from the tray still works
/// while it is open. The error dialog stays a blocking NSAlert.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelResult {
    Cancel,
    Apply,
}

struct PortPanel {
    window: Retained<NSPanel>,
    field: Retained<NSTextField>,
    current: u16,
    result: Arc<Mutex<Option<PanelResult>>>,
    _watcher: Retained<PortWatcher>,
    _actions: Retained<PanelActions>,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(marker) = MainThreadMarker::new() else {
        return Err("reg-server desk must run on the main thread".into());
    };
    let app = NSApplication::sharedApplication(marker);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

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
    let probe_letter = server::upgrade_probe_letter();
    let (width, height, rgba) = menu_icon();
    let icon = tray_icon::Icon::from_rgba(rgba, width, height)?;
    let mut shown = session.address_text();
    let address = MenuItem::with_id(
        "address",
        &shown,
        server::listen_url(&shown).is_some(),
        None,
    );
    let port_item = MenuItem::with_id("port", "Port…", true, None);
    let probe_item = if server::show_upgrade_probe_menu(probe_letter) {
        Some(MenuItem::with_id(
            "upgrade-probe",
            server::UPGRADE_PROBE_MENU,
            true,
            None,
        ))
    } else {
        None
    };
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu = Menu::new();
    menu.append(&address)?;
    menu.append(&port_item)?;
    if let Some(ref item) = probe_item {
        menu.append(item)?;
    }
    menu.append(&quit)?;
    let tooltip = match probe_letter {
        Some(letter) => format!("reg-server (probe {letter})"),
        None => "reg-server".into(),
    };
    let builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .with_tooltip(&tooltip);
    let _tray = if server::probe_uses_color_icon(probe_letter) {
        builder.with_icon(icon).build()?
    } else {
        builder.with_icon_templated(icon).build()?
    };

    let address_id = address.id().clone();
    let port_id = port_item.id().clone();
    let probe_id = probe_item.as_ref().map(|item| item.id().clone());
    let quit_id = quit.id().clone();
    let mut panel = None;
    if let Some(message) = session.failure() {
        show_error(marker, &app, &message);
    }

    loop {
        while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(NSDate::distantPast().as_ref()),
            unsafe { NSDefaultRunLoopMode },
            true,
        ) {
            app.sendEvent(&event);
        }

        if panel_closed(&panel) {
            close_panel(&app, &mut panel);
        } else if let Some(choice) = panel_result(&panel) {
            match choice {
                PanelResult::Apply => {
                    let port = panel.as_ref().and_then(|open| {
                        server::replacement_port(
                            open.current,
                            &open.field.stringValue().to_string(),
                        )
                    });
                    close_panel(&app, &mut panel);
                    if let Some(port) = port {
                        session.restart(port);
                        shown = session.address_text();
                        set_address_item(&address, &shown);
                        if let Some(message) = session.failure() {
                            show_error(marker, &app, &message);
                        }
                    }
                }
                PanelResult::Cancel => close_panel(&app, &mut panel),
            }
        }

        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == quit_id {
                close_panel(&app, &mut panel);
                session.stop();
                break;
            }
            if event.id == address_id {
                server::open_listen(&session.address_text());
            }
            if event.id == port_id {
                match panel.as_ref() {
                    Some(open) => bring_panel_front(&app, &open.window),
                    None => panel = Some(open_port(marker, &app, session.port())),
                }
            }
            if probe_id.as_ref() == Some(&event.id) {
                server::desk_log("action UpgradeProbe");
                show_upgrade_probe(marker, &app);
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

fn show_upgrade_probe(marker: MainThreadMarker, app: &NSApplication) {
    let alert = NSAlert::new(marker);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str(server::UPGRADE_PROBE_MENU));
    alert.setInformativeText(&NSString::from_str("Tray marker B is active."));
    activate_app(app);
    alert.layout();
    let _ = alert.runModal();
}

/// Blocking error alert. Quit from the tray does not dismiss it.
fn show_error(marker: MainThreadMarker, app: &NSApplication, message: &str) {
    let alert = NSAlert::new(marker);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str("reg-server"));
    alert.setInformativeText(&NSString::from_str(message));
    activate_app(app);
    alert.layout();
    let window = alert.window();
    window.setHidesOnDeactivate(false);
    window.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace);
    window.setLevel(NSModalPanelWindowLevel);
    window.center();
    window.makeKeyAndOrderFront(None);
    let _ = alert.runModal();
    window.orderOut(None);
    window.close();
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

fn panel_result(panel: &Option<PortPanel>) -> Option<PanelResult> {
    panel
        .as_ref()
        .and_then(|open| open.result.lock().expect("panel").take())
}

fn panel_closed(panel: &Option<PortPanel>) -> bool {
    panel.as_ref().is_some_and(|open| !open.window.isVisible())
}

fn close_panel(app: &NSApplication, panel: &mut Option<PortPanel>) {
    if let Some(open) = panel.take() {
        open.window.orderOut(None);
        open.window.close();
    }
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

fn open_port(marker: MainThreadMarker, app: &NSApplication, current: u16) -> PortPanel {
    let result = Arc::new(Mutex::new(None));
    let actions = PanelActions::new(marker, Arc::clone(&result));
    let width = 320.0;
    let height = 140.0;
    let window = make_panel(marker, "Port", width, height);
    let content = window.contentView().expect("content");

    let label = NSTextField::labelWithString(&NSString::from_str("Port"), marker);
    label.setFrame(NSRect {
        origin: NSPoint { x: 20.0, y: 90.0 },
        size: NSSize {
            width: width - 40.0,
            height: 20.0,
        },
    });
    content.addSubview(&label);

    let field = NSTextField::textFieldWithString(&NSString::from_str(&current.to_string()), marker);
    field.setFrame(NSRect {
        origin: NSPoint { x: 20.0, y: 58.0 },
        size: NSSize {
            width: width - 40.0,
            height: 24.0,
        },
    });
    content.addSubview(&field);

    let cancel = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Cancel"),
            Some(&*actions),
            Some(sel!(cancel:)),
            marker,
        )
    };
    cancel.setFrame(NSRect {
        origin: NSPoint { x: 20.0, y: 16.0 },
        size: NSSize {
            width: 90.0,
            height: 28.0,
        },
    });
    content.addSubview(&cancel);

    let apply = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Apply and restart"),
            Some(&*actions),
            Some(sel!(apply:)),
            marker,
        )
    };
    apply.setEnabled(false);
    apply.setFrame(NSRect {
        origin: NSPoint {
            x: width - 160.0,
            y: 16.0,
        },
        size: NSSize {
            width: 140.0,
            height: 28.0,
        },
    });
    content.addSubview(&apply);

    let watcher = PortWatcher::new(marker, Retained::clone(&field), Retained::clone(&apply), current);
    unsafe {
        field.setDelegate(Some(ProtocolObject::from_ref(&*watcher)));
    }

    bring_panel_front(app, &window);
    PortPanel {
        window,
        field,
        current,
        result,
        _watcher: watcher,
        _actions: actions,
    }
}

fn make_panel(marker: MainThreadMarker, title: &str, width: f64, height: f64) -> Retained<NSPanel> {
    let window = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(marker),
        NSRect {
            origin: NSPoint { x: 0.0, y: 0.0 },
            size: NSSize { width, height },
        },
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
        NSBackingStoreType::Buffered,
        false,
    );
    window.setTitle(&NSString::from_str(title));
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    window.setHidesOnDeactivate(false);
    window.setFloatingPanel(true);
    window.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace);
    window.setLevel(NSModalPanelWindowLevel);
    window
}

#[allow(deprecated)]
fn activate_app(app: &NSApplication) {
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.unhide(None);
    let running = NSRunningApplication::currentApplication();
    let _ = running.unhide();
    let _ = running.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
    app.activateIgnoringOtherApps(true);
}

#[allow(deprecated)]
fn bring_panel_front(app: &NSApplication, window: &NSPanel) {
    activate_app(app);
    window.center();
    window.makeKeyAndOrderFront(None);
    window.orderFrontRegardless();
}

struct PortIvars {
    field: Retained<NSTextField>,
    apply: Retained<NSButton>,
    current: u16,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PortIvars]
    struct PortWatcher;

    unsafe impl NSObjectProtocol for PortWatcher {}

    unsafe impl NSControlTextEditingDelegate for PortWatcher {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _note: &NSNotification) {
            let ivars = self.ivars();
            let enabled =
                server::replacement_port(ivars.current, &ivars.field.stringValue().to_string())
                    .is_some();
            ivars.apply.setEnabled(enabled);
        }
    }

    unsafe impl NSTextFieldDelegate for PortWatcher {}
);

impl PortWatcher {
    fn new(
        marker: MainThreadMarker,
        field: Retained<NSTextField>,
        apply: Retained<NSButton>,
        current: u16,
    ) -> Retained<Self> {
        let this = Self::alloc(marker).set_ivars(PortIvars {
            field,
            apply,
            current,
        });
        unsafe { msg_send![super(this), init] }
    }
}

struct PanelIvars {
    result: Arc<Mutex<Option<PanelResult>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PanelIvars]
    struct PanelActions;

    unsafe impl NSObjectProtocol for PanelActions {}

    impl PanelActions {
        #[unsafe(method(cancel:))]
        fn cancel(&self, _sender: Option<&AnyObject>) {
            *self.ivars().result.lock().expect("panel") = Some(PanelResult::Cancel);
        }

        #[unsafe(method(apply:))]
        fn apply(&self, _sender: Option<&AnyObject>) {
            *self.ivars().result.lock().expect("panel") = Some(PanelResult::Apply);
        }
    }
);

impl PanelActions {
    fn new(marker: MainThreadMarker, result: Arc<Mutex<Option<PanelResult>>>) -> Retained<Self> {
        let this = Self::alloc(marker).set_ivars(PanelIvars { result });
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) fn menu_icon() -> (u32, u32, Vec<u8>) {
    menu_icon_for(server::upgrade_probe_letter())
}

pub(crate) fn menu_icon_for(letter: Option<char>) -> (u32, u32, Vec<u8>) {
    match system_pencil_alpha() {
        Some(alpha) if server::probe_uses_color_icon(letter) => {
            server::icon_with_pencil_rgb(&alpha, server::probe_icon_rgb(letter))
        }
        Some(alpha) => server::icon_with_pencil(&alpha),
        None => server::icon_rgba_for(letter),
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

