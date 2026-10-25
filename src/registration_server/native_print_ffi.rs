// C ABI. The symbols are the same on Linux, macOS, and Windows.
// The shared library contains the CUPS client or the spooler client for the target.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

pub struct NativePrintList {
    queues: Vec<OwnedQueue>,
}

struct OwnedQueue {
    name: CString,
    device_uri: CString,
    make_model: CString,
    state: CString,
    host: CString,
    kind: CString,
    width_in: CString,
    accepting: c_int,
    dpi: c_int,
    installed: c_int,
    connected: c_int,
    choices: Vec<OwnedChoice>,
}

struct OwnedChoice {
    name: CString,
    values: Vec<CString>,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").unwrap());
}

fn set_message(message: &str) {
    let cleaned = message.replace('\0', " ");
    let text = CString::new(cleaned).unwrap_or_else(|_| CString::new("error").unwrap());
    LAST_ERROR.with(|slot| *slot.borrow_mut() = text);
}

fn clear_error() {
    set_message("");
}

fn failure(message: &str) -> c_int {
    set_message(message);
    1
}

fn guard(body: impl FnOnce() -> c_int) -> c_int {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(code) => code,
        Err(_) => failure("panic"),
    }
}

fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).unwrap_or_else(|_| CString::new("").unwrap())
}

fn own(queue: Queue) -> OwnedQueue {
    OwnedQueue {
        name: c_string(&queue.name),
        device_uri: c_string(&queue.device_uri),
        make_model: c_string(&queue.make_model),
        state: c_string(&queue.state),
        host: c_string(&queue.host),
        kind: c_string(queue.kind.as_str()),
        width_in: c_string(&width_text(queue.width_in)),
        accepting: c_int::from(queue.accepting),
        dpi: c_int::from(queue.dpi),
        installed: c_int::from(queue.installed),
        connected: c_int::from(queue.connected),
        choices: queue
            .choices
            .into_iter()
            .map(|choice| OwnedChoice {
                name: c_string(&choice.name),
                values: choice.values.iter().map(|value| c_string(value)).collect(),
            })
            .collect(),
    }
}

fn width_text(width: Option<f64>) -> String {
    match width {
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

fn service_text(service: *const c_char) -> Result<Option<String>, c_int> {
    if service.is_null() {
        return Ok(None);
    }
    match unsafe { CStr::from_ptr(service) }.to_str() {
        Ok(text) if text.is_empty() => Ok(None),
        Ok(text) => Ok(Some(text.to_string())),
        Err(_) => Err(failure("service is not utf-8")),
    }
}

fn queue_at(list: *const NativePrintList, index: usize) -> Option<*const OwnedQueue> {
    if list.is_null() {
        return None;
    }
    let list = unsafe { &*list };
    list.queues
        .get(index)
        .map(|queue| queue as *const OwnedQueue)
}

fn text_at(
    list: *const NativePrintList,
    index: usize,
    field: fn(&OwnedQueue) -> &CString,
) -> *const c_char {
    match queue_at(list, index) {
        Some(queue) => field(unsafe { &*queue }).as_ptr(),
        None => ptr::null(),
    }
}

#[no_mangle]
pub extern "C" fn native_print_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn native_print_list(
    service: *const c_char,
    out: *mut *mut NativePrintList,
    count: *mut usize,
) -> c_int {
    guard(|| {
        if out.is_null() || count.is_null() {
            return failure("missing output");
        }
        unsafe {
            *out = ptr::null_mut();
            *count = 0;
        }
        let service = match service_text(service) {
            Ok(service) => service,
            Err(code) => return code,
        };
        let queues = match list_service(service.as_deref()) {
            Ok(queues) => queues,
            Err(err) => return failure(&err.to_string()),
        };
        let len = queues.len();
        let list = Box::new(NativePrintList {
            queues: queues.into_iter().map(own).collect(),
        });
        unsafe {
            *out = Box::into_raw(list);
            *count = len;
        }
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn native_print_list_free(list: *mut NativePrintList) {
    if list.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(list) });
}

#[no_mangle]
pub extern "C" fn native_print_name(list: *const NativePrintList, index: usize) -> *const c_char {
    text_at(list, index, |queue| &queue.name)
}

#[no_mangle]
pub extern "C" fn native_print_device_uri(
    list: *const NativePrintList,
    index: usize,
) -> *const c_char {
    text_at(list, index, |queue| &queue.device_uri)
}

#[no_mangle]
pub extern "C" fn native_print_make_model(
    list: *const NativePrintList,
    index: usize,
) -> *const c_char {
    text_at(list, index, |queue| &queue.make_model)
}

#[no_mangle]
pub extern "C" fn native_print_state(list: *const NativePrintList, index: usize) -> *const c_char {
    text_at(list, index, |queue| &queue.state)
}

#[no_mangle]
pub extern "C" fn native_print_host(list: *const NativePrintList, index: usize) -> *const c_char {
    text_at(list, index, |queue| &queue.host)
}

#[no_mangle]
pub extern "C" fn native_print_kind(list: *const NativePrintList, index: usize) -> *const c_char {
    text_at(list, index, |queue| &queue.kind)
}

#[no_mangle]
pub extern "C" fn native_print_width_in(
    list: *const NativePrintList,
    index: usize,
) -> *const c_char {
    text_at(list, index, |queue| &queue.width_in)
}

#[no_mangle]
pub extern "C" fn native_print_accepting(list: *const NativePrintList, index: usize) -> c_int {
    queue_at(list, index)
        .map(|queue| unsafe { &*queue }.accepting)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn native_print_dpi(list: *const NativePrintList, index: usize) -> c_int {
    queue_at(list, index)
        .map(|queue| unsafe { &*queue }.dpi)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn native_print_installed(list: *const NativePrintList, index: usize) -> c_int {
    queue_at(list, index)
        .map(|queue| unsafe { &*queue }.installed)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn native_print_connected(list: *const NativePrintList, index: usize) -> c_int {
    queue_at(list, index)
        .map(|queue| unsafe { &*queue }.connected)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn native_print_choice_count(list: *const NativePrintList, index: usize) -> usize {
    queue_at(list, index)
        .map(|queue| unsafe { &*queue }.choices.len())
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn native_print_choice_name(
    list: *const NativePrintList,
    index: usize,
    choice: usize,
) -> *const c_char {
    let Some(queue) = queue_at(list, index) else {
        return ptr::null();
    };
    match unsafe { &*queue }.choices.get(choice) {
        Some(choice) => choice.name.as_ptr(),
        None => ptr::null(),
    }
}

#[no_mangle]
pub extern "C" fn native_print_choice_value_count(
    list: *const NativePrintList,
    index: usize,
    choice: usize,
) -> usize {
    let Some(queue) = queue_at(list, index) else {
        return 0;
    };
    unsafe { &*queue }
        .choices
        .get(choice)
        .map(|choice| choice.values.len())
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn native_print_choice_value(
    list: *const NativePrintList,
    index: usize,
    choice: usize,
    value: usize,
) -> *const c_char {
    let Some(queue) = queue_at(list, index) else {
        return ptr::null();
    };
    unsafe { &*queue }
        .choices
        .get(choice)
        .and_then(|choice| choice.values.get(value))
        .map(|value| value.as_ptr())
        .unwrap_or(ptr::null())
}

#[cfg(test)]
mod abi {
    use std::ffi::{CStr, CString};
    use std::ptr;

    use super::{
        native_print_connected, native_print_device_uri, native_print_dpi, native_print_host,
        native_print_installed, native_print_kind, native_print_last_error, native_print_list,
        native_print_list_free, native_print_name, native_print_width_in, NativePrintList,
    };

    fn c_str(text: &str) -> CString {
        CString::new(text).unwrap()
    }

    fn text(ptr: *const std::os::raw::c_char) -> String {
        assert!(!ptr.is_null());
        unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string()
    }

    fn last_error() -> String {
        text(native_print_last_error())
    }

    #[test]
    fn c_abi_lists_the_queues() {
        let missing = native_print_list(ptr::null(), ptr::null_mut(), ptr::null_mut());
        assert_eq!(missing, 1);
        assert!(last_error().contains("output"));

        let listed = Listed::open();
        let mut out: *mut NativePrintList = ptr::null_mut();
        let mut count = 0usize;
        let code = native_print_list(listed.service_ptr(), &mut out, &mut count);
        assert_eq!(code, 0, "{}", last_error());
        assert!(count >= 2, "{count}");
        let zebra = find(&out, count, "zebra");
        assert_eq!(text(native_print_kind(out, zebra)), "zpl");
        assert_eq!(native_print_dpi(out, zebra), 300);
        assert_eq!(text(native_print_host(out, zebra)), "");
        assert_eq!(native_print_installed(out, zebra), 1);
        assert_eq!(native_print_connected(out, zebra), 0);
        let ipp = find(&out, count, "ipp");
        assert_eq!(text(native_print_host(out, ipp)), "192.0.2.10");
        assert_eq!(
            text(native_print_device_uri(out, ipp)),
            "ipp://192.0.2.10/ipp/print"
        );
        assert_eq!(text(native_print_width_in(out, zebra)), "4.1");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let evolis = find(&out, count, "evolis");
            assert_eq!(text(native_print_kind(out, evolis)), "raster");
            assert_eq!(text(native_print_width_in(out, evolis)), "2.125");
        }
        native_print_list_free(out);
        listed.close();
    }

    fn find(list: &*mut NativePrintList, count: usize, name: &str) -> usize {
        (0..count)
            .find(|index| text(native_print_name(*list, *index)) == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    }

    struct Listed {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        service: CString,
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Listed {
        fn open() -> Self {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                let (base, handle) = serve_sample();
                Self {
                    service: c_str(&base),
                    handle: Some(handle),
                }
            }
            #[cfg(windows)]
            {
                super::SPOOLER_FIXTURE.with(|slot| {
                    *slot.borrow_mut() = Some(vec![
                        super::SpoolerRow {
                            name: "ipp".to_string(),
                            driver: "HP LaserJet".to_string(),
                            port: "ipp://192.0.2.10/ipp/print".to_string(),
                            status: "idle".to_string(),
                            jobs: 0,
                        },
                        super::SpoolerRow {
                            name: "zebra".to_string(),
                            driver: "Zebra Technologies ZTC GX430t".to_string(),
                            port: "USB001".to_string(),
                            status: "idle".to_string(),
                            jobs: 0,
                        },
                    ]);
                });
                Self {}
            }
        }

        fn service_ptr(&self) -> *const std::os::raw::c_char {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.service.as_ptr()
            }
            #[cfg(windows)]
            {
                ptr::null()
            }
        }

        fn close(self) {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                if let Some(handle) = self.handle {
                    let _ = handle.join();
                }
            }
            #[cfg(windows)]
            {
                super::SPOOLER_FIXTURE.with(|slot| *slot.borrow_mut() = None);
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn serve_sample() -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        let response = sample_response();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                if request_complete(&buf) {
                    break;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            sock.write_all(head.as_bytes()).unwrap();
            sock.write_all(&response).unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn request_complete(buf: &[u8]) -> bool {
        let Some(end) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let Ok(head) = std::str::from_utf8(&buf[..end]) else {
            return false;
        };
        let Some(length) = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        }) else {
            return false;
        };
        buf.len() >= end + 4 + length
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn sample_response() -> Vec<u8> {
        let printers = [
            (
                "zebra",
                "usb://Zebra/ZTC?serial=ABC",
                "Zebra Technologies ZTC GX430t",
                3,
                true,
            ),
            ("ipp", "ipp://192.0.2.10/ipp/print", "HP LaserJet", 4, true),
            (
                "evolis",
                "usb://Evolis/Zenius?serial=E1",
                "Evolis Zenius",
                5,
                false,
            ),
        ];
        let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
        write_text(&mut out, 0x47, "attributes-charset", "utf-8");
        write_text(&mut out, 0x48, "attributes-natural-language", "en");
        for (name, uri, make, state, accepting) in printers {
            out.push(0x04);
            write_text(&mut out, 0x42, "printer-name", name);
            write_text(&mut out, 0x45, "device-uri", uri);
            write_text(&mut out, 0x41, "printer-make-and-model", make);
            write_enum(&mut out, "printer-state", state);
            write_bool(&mut out, "printer-is-accepting-jobs", accepting);
        }
        out.push(0x03);
        out
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn write_text(out: &mut Vec<u8>, tag: u8, name: &str, value: &str) {
        out.push(tag);
        counted(out, name.as_bytes());
        counted(out, value.as_bytes());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn write_enum(out: &mut Vec<u8>, name: &str, value: i32) {
        out.push(0x23);
        counted(out, name.as_bytes());
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&value.to_be_bytes());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn write_bool(out: &mut Vec<u8>, name: &str, value: bool) {
        out.push(0x22);
        counted(out, name.as_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.push(u8::from(value));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn counted(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        out.extend_from_slice(bytes);
    }

    #[test]
    fn python_embed_lists_the_queues() {
        let listed = Listed::open();
        let library = shared_library();
        assert!(library.is_file(), "missing {}", library.display());
        let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("modules/native-print/tests/test_embed.py");
        let mut command = python_command();
        command.arg(&script).env("NATIVE_PRINT_LIB", &library);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            command.env("NATIVE_PRINT_SERVICE", listed.service.to_str().unwrap());
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let dead = listener.local_addr().unwrap();
            drop(listener);
            command.env("NATIVE_PRINT_DEAD", format!("http://{dead}"));
        }
        let status = command.status().expect("python3");
        assert!(status.success());
        listed.close();
    }

    fn python_command() -> std::process::Command {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            let mut command = std::process::Command::new("arch");
            command.args(["-arm64", "/usr/bin/python3"]);
            command
        } else {
            std::process::Command::new("python3")
        }
    }

    fn shared_library() -> std::path::PathBuf {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = std::process::Command::new(cargo)
            .args(["build", "--lib"])
            .status()
            .expect("cargo build --lib");
        assert!(status.success());
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let filename = if cfg!(target_os = "macos") {
            "librust_reg.dylib"
        } else if cfg!(target_os = "windows") {
            "rust_reg.dll"
        } else {
            "librust_reg.so"
        };
        target.join(profile).join(filename)
    }
}
