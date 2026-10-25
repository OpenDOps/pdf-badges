use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use crate::{Catalog, MdnsBrowse, Printer};

pub struct IppPrinter {
    printer: Printer,
    make_model: CString,
    jobs: Option<(*mut IppJob, usize)>,
}

/// The fields are read by the tests. A non-test build only returns the pointer.
#[cfg_attr(not(test), allow(dead_code))]
pub struct IppJob {
    pub id: i32,
    pub state: i32,
    pub name: CString,
    pub reasons: CString,
    pub impressions: i32,
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

fn c_text<'a>(ptr: *const c_char, what: &str) -> Result<&'a str, c_int> {
    if ptr.is_null() {
        return Err(failure(&format!("missing {what}")));
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(text) => Ok(text),
        Err(_) => Err(failure(&format!("{what} is not utf-8"))),
    }
}

fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).unwrap_or_else(|_| CString::new("").unwrap())
}

fn printer_handle(printer: Printer) -> Box<IppPrinter> {
    let make_model = c_string(&printer.make_model);
    Box::new(IppPrinter {
        printer,
        make_model,
        jobs: None,
    })
}

fn write_list(printers: Vec<Printer>, out: *mut *mut *mut IppPrinter, count: *mut usize) {
    if printers.is_empty() {
        unsafe {
            *out = ptr::null_mut();
            *count = 0;
        }
        return;
    }
    let mut list: Vec<*mut IppPrinter> = printers
        .into_iter()
        .map(|printer| Box::into_raw(printer_handle(printer)))
        .collect();
    let len = list.len();
    let ptr = list.as_mut_ptr();
    std::mem::forget(list);
    unsafe {
        *out = ptr;
        *count = len;
    }
}

fn printer_mut<'a>(printer: *mut IppPrinter) -> Result<&'a mut IppPrinter, c_int> {
    if printer.is_null() {
        Err(failure("missing printer"))
    } else {
        Ok(unsafe { &mut *printer })
    }
}

fn write_job_id(slot: *mut c_int, id: i32) -> c_int {
    unsafe {
        *slot = id;
    }
    clear_error();
    0
}

fn job_call(
    printer: *mut IppPrinter,
    job_id: c_int,
    call: impl FnOnce(&Printer, i32) -> Result<(), crate::Error>,
) -> c_int {
    let printer = match printer_mut(printer) {
        Ok(printer) => printer,
        Err(code) => return code,
    };
    match call(&printer.printer, job_id) {
        Ok(()) => {
            clear_error();
            0
        }
        Err(err) => failure(err.message()),
    }
}

fn free_jobs(jobs: &mut Option<(*mut IppJob, usize)>) {
    if let Some((ptr, len)) = jobs.take() {
        if !ptr.is_null() && len > 0 {
            unsafe {
                drop(Vec::from_raw_parts(ptr, len, len));
            }
        }
    }
}

impl Drop for IppPrinter {
    fn drop(&mut self) {
        free_jobs(&mut self.jobs);
    }
}

#[no_mangle]
pub extern "C" fn ipp_tool_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn ipp_tool_printer_open(uri: *const c_char, out: *mut *mut IppPrinter) -> c_int {
    guard(|| {
        if out.is_null() {
            return failure("missing output");
        }
        unsafe {
            *out = ptr::null_mut();
        }
        let uri = match c_text(uri, "uri") {
            Ok(uri) => uri,
            Err(code) => return code,
        };
        let printer = match Printer::from_uri(uri) {
            Ok(printer) => printer,
            Err(err) => return failure(err.message()),
        };
        let handle = printer_handle(printer);
        unsafe {
            *out = Box::into_raw(handle);
        }
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_info(printer: *mut IppPrinter) -> c_int {
    guard(|| {
        if printer.is_null() {
            return failure("missing printer");
        }
        let printer = unsafe { &mut *printer };
        if let Err(err) = printer.printer.info() {
            return failure(err.message());
        }
        printer.make_model = c_string(&printer.printer.make_model);
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_make_model(printer: *const IppPrinter) -> *const c_char {
    if printer.is_null() {
        return ptr::null();
    }
    unsafe { &*printer }.make_model.as_ptr()
}

#[no_mangle]
pub extern "C" fn ipp_tool_set(
    printer: *mut IppPrinter,
    setting: *const c_char,
    value: *const c_char,
) -> c_int {
    guard(|| {
        if printer.is_null() {
            return failure("missing printer");
        }
        let setting = match c_text(setting, "setting") {
            Ok(setting) => setting,
            Err(code) => return code,
        };
        let value = match c_text(value, "value") {
            Ok(value) => value,
            Err(code) => return code,
        };
        let printer = unsafe { &mut *printer };
        if let Err(err) = printer.printer.set(setting, value) {
            return failure(err.message());
        }
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_print_test(printer: *mut IppPrinter, job_id: *mut c_int) -> c_int {
    guard(|| {
        if job_id.is_null() {
            return failure("missing job id");
        }
        let printer = match printer_mut(printer) {
            Ok(printer) => printer,
            Err(code) => return code,
        };
        match printer.printer.print_test() {
            Ok(id) => write_job_id(job_id, id),
            Err(err) => failure(err.message()),
        }
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_print_file(
    printer: *mut IppPrinter,
    path: *const c_char,
    raw: c_int,
    job_id: *mut c_int,
) -> c_int {
    guard(|| {
        if job_id.is_null() {
            return failure("missing job id");
        }
        let path = match c_text(path, "path") {
            Ok(path) => path,
            Err(code) => return code,
        };
        let printer = match printer_mut(printer) {
            Ok(printer) => printer,
            Err(code) => return code,
        };
        match printer.printer.print_file(path, raw != 0) {
            Ok(id) => write_job_id(job_id, id),
            Err(err) => failure(err.message()),
        }
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_jobs(
    printer: *mut IppPrinter,
    out: *mut *mut IppJob,
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
        let printer = match printer_mut(printer) {
            Ok(printer) => printer,
            Err(code) => return code,
        };
        let jobs = match printer.printer.jobs() {
            Ok(jobs) => jobs,
            Err(err) => return failure(err.message()),
        };
        free_jobs(&mut printer.jobs);
        if jobs.is_empty() {
            clear_error();
            return 0;
        }
        let mut block: Vec<IppJob> = jobs
            .into_iter()
            .map(|job| IppJob {
                id: job.id,
                state: job.state,
                name: c_string(&job.name),
                reasons: c_string(&job.reasons.join(",")),
                impressions: job.impressions_completed,
            })
            .collect();
        let len = block.len();
        let ptr = block.as_mut_ptr();
        std::mem::forget(block);
        printer.jobs = Some((ptr, len));
        unsafe {
            *out = ptr;
            *count = len;
        }
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_cancel(printer: *mut IppPrinter, job_id: c_int) -> c_int {
    guard(|| job_call(printer, job_id, |printer, id| printer.cancel_job(id)))
}

#[no_mangle]
pub extern "C" fn ipp_tool_hold(printer: *mut IppPrinter, job_id: c_int) -> c_int {
    guard(|| job_call(printer, job_id, |printer, id| printer.hold_job(id)))
}

#[no_mangle]
pub extern "C" fn ipp_tool_release(printer: *mut IppPrinter, job_id: c_int) -> c_int {
    guard(|| job_call(printer, job_id, |printer, id| printer.release_job(id)))
}

#[no_mangle]
pub extern "C" fn ipp_tool_discover(out: *mut *mut *mut IppPrinter, count: *mut usize) -> c_int {
    guard(|| {
        if out.is_null() || count.is_null() {
            return failure("missing output");
        }
        unsafe {
            *out = ptr::null_mut();
            *count = 0;
        }
        let catalog = match Catalog::discover(&MdnsBrowse) {
            Ok(catalog) => catalog,
            Err(err) => return failure(err.message()),
        };
        write_list(catalog.printers().to_vec(), out, count);
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_refresh(
    printers: *mut *mut *mut IppPrinter,
    count: *mut usize,
) -> c_int {
    guard(|| {
        if printers.is_null() || count.is_null() {
            return failure("missing output");
        }
        let current = unsafe { *printers };
        let current_count = unsafe { *count };
        let owned = if current.is_null() || current_count == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(current, current_count) }
                .iter()
                .map(|printer| unsafe { (**printer).printer.clone() })
                .collect()
        };
        let mut catalog = Catalog::from_printers(owned);
        if let Err(err) = catalog.refresh(&MdnsBrowse) {
            return failure(err.message());
        }
        if !current.is_null() && current_count > 0 {
            ipp_tool_list_free(current, current_count);
        }
        write_list(catalog.printers().to_vec(), printers, count);
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_add(uri: *const c_char, out: *mut *mut IppPrinter) -> c_int {
    guard(|| {
        if out.is_null() {
            return failure("missing output");
        }
        unsafe {
            *out = ptr::null_mut();
        }
        let uri = match c_text(uri, "uri") {
            Ok(uri) => uri,
            Err(code) => return code,
        };
        let mut catalog = Catalog::default();
        if let Err(err) = catalog.add(uri) {
            return failure(err.message());
        }
        let printer = match catalog.printers().first() {
            Some(printer) => printer.clone(),
            None => return failure("no printer"),
        };
        unsafe {
            *out = Box::into_raw(printer_handle(printer));
        }
        clear_error();
        0
    })
}

#[no_mangle]
pub extern "C" fn ipp_tool_printer_free(printer: *mut IppPrinter) {
    if printer.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(printer));
    }
}

#[no_mangle]
pub extern "C" fn ipp_tool_list_free(list: *mut *mut IppPrinter, count: usize) {
    if list.is_null() || count == 0 {
        return;
    }
    unsafe {
        let printers = Vec::from_raw_parts(list, count, count);
        for printer in printers {
            ipp_tool_printer_free(printer);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::message::{decode, encode_request, Operation, Value};

    fn zebra_response() -> Vec<u8> {
        encode_request(
            Operation::GetPrinterAttributes,
            1,
            &[],
            &[
                (
                    "printer-make-and-model",
                    vec![Value::Name("Zebra".to_string())],
                ),
                (
                    "printer-settable-attributes-supported",
                    vec![Value::Keyword("media".to_string())],
                ),
                (
                    "sides-supported",
                    vec![
                        Value::Keyword("one-sided".to_string()),
                        Value::Keyword("two-sided-long-edge".to_string()),
                    ],
                ),
                (
                    "sides-default",
                    vec![Value::Keyword("one-sided".to_string())],
                ),
            ],
        )
    }

    fn serve(ipp: Vec<u8>) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                if http_body(&request).is_some() {
                    break;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&tmp[..n]),
                }
            }
            let _ = tx.send(request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                ipp.len()
            );
            let _ = sock.write_all(header.as_bytes());
            let _ = sock.write_all(&ipp);
        });
        (port, rx)
    }

    fn http_body(buf: &[u8]) -> Option<Vec<u8>> {
        let split = buf.windows(4).position(|window| window == b"\r\n\r\n")?;
        let head = std::str::from_utf8(&buf[..split]).ok()?;
        let length = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })?;
        let start = split + 4;
        if buf.len() < start + length {
            return None;
        }
        Some(buf[start..start + length].to_vec())
    }

    fn last_error() -> String {
        unsafe { CStr::from_ptr(ipp_tool_last_error()) }
            .to_str()
            .unwrap()
            .to_string()
    }

    fn open_on(port: u16) -> *mut IppPrinter {
        let uri = CString::new(format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        let mut printer = std::ptr::null_mut();
        assert_eq!(ipp_tool_printer_open(uri.as_ptr(), &mut printer), 0);
        assert!(!printer.is_null());
        printer
    }

    #[test]
    fn c_abi_info_reads_the_make() {
        let (port, _rx) = serve(zebra_response());
        let printer = open_on(port);
        assert_eq!(ipp_tool_info(printer), 0);
        let make = unsafe { CStr::from_ptr(ipp_tool_make_model(printer)) };
        assert_eq!(make.to_str().unwrap(), "Zebra");
        ipp_tool_printer_free(printer);
    }

    #[test]
    fn c_abi_readonly_set_sends_nothing() {
        let (port, rx) = serve(zebra_response());
        let printer = open_on(port);
        assert_eq!(ipp_tool_info(printer), 0);
        let setting = CString::new("sides").unwrap();
        let value = CString::new("two-sided-long-edge").unwrap();
        let code = ipp_tool_set(printer, setting.as_ptr(), value.as_ptr());
        assert_ne!(code, 0);
        assert!(
            last_error().contains("sides"),
            "error should name sides, got {}",
            last_error()
        );
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let body = http_body(&raw).unwrap();
        let message = decode(&body).unwrap();
        assert_eq!(message.operation(), Some(Operation::GetPrinterAttributes));
        ipp_tool_printer_free(printer);
    }

    fn serve_seq(responses: Vec<Vec<u8>>) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for ipp in responses {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    if http_body(&request).is_some() {
                        break;
                    }
                    match sock.read(&mut tmp) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&tmp[..n]),
                    }
                }
                let _ = tx.send(request);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    ipp.len()
                );
                let _ = sock.write_all(header.as_bytes());
                let _ = sock.write_all(&ipp);
            }
        });
        (port, rx)
    }

    fn formats_response() -> Vec<u8> {
        encode_request(
            Operation::GetPrinterAttributes,
            1,
            &[],
            &[
                (
                    "printer-make-and-model",
                    vec![Value::Name("Zebra".to_string())],
                ),
                (
                    "document-format-supported",
                    vec![Value::Mime("application/pdf".to_string())],
                ),
                (
                    "document-format-preferred",
                    vec![Value::Mime("application/pdf".to_string())],
                ),
            ],
        )
    }

    fn job_response(id: i32) -> Vec<u8> {
        let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
        out.push(0x02);
        out.push(0x21);
        let name = b"job-id";
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&id.to_be_bytes());
        out.push(0x03);
        out
    }

    #[test]
    fn c_abi_print_test_sends_pdf() {
        let (port, rx) = serve_seq(vec![formats_response(), job_response(4)]);
        let printer = open_on(port);
        assert_eq!(ipp_tool_info(printer), 0);
        let _ = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut job_id = 0;
        assert_eq!(ipp_tool_print_test(printer, &mut job_id), 0);
        assert_eq!(job_id, 4);
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(raw.windows(4).any(|window| window == b"%PDF"));
        ipp_tool_printer_free(printer);
    }

    #[test]
    fn c_abi_cancel_sends_cancel_job() {
        let (port, rx) = serve(crate::message::encode_response(0, 1));
        let printer = open_on(port);
        assert_eq!(ipp_tool_cancel(printer, 7), 0);
        let raw = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let body = http_body(&raw).unwrap();
        let message = decode(&body).unwrap();
        assert_eq!(message.operation(), Some(Operation::CancelJob));
        assert_eq!(message.texts("job-id"), ["7"]);
        ipp_tool_printer_free(printer);
    }

    #[test]
    fn c_abi_hold_and_release_send_the_job() {
        let (port, rx) = serve_seq(vec![
            crate::message::encode_response(0, 1),
            crate::message::encode_response(0, 1),
        ]);
        let printer = open_on(port);
        assert_eq!(ipp_tool_hold(printer, 3), 0);
        assert_eq!(ipp_tool_release(printer, 3), 0);
        let held = http_body(&rx.recv_timeout(Duration::from_secs(2)).unwrap()).unwrap();
        let released = http_body(&rx.recv_timeout(Duration::from_secs(2)).unwrap()).unwrap();
        assert_eq!(decode(&held).unwrap().operation(), Some(Operation::HoldJob));
        assert_eq!(
            decode(&released).unwrap().operation(),
            Some(Operation::ReleaseJob)
        );
        ipp_tool_printer_free(printer);
    }

    fn jobs_response(id: i32, state: i32) -> Vec<u8> {
        let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
        out.push(0x02);
        out.push(0x21);
        let name = b"job-id";
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&id.to_be_bytes());
        out.push(0x23);
        let state_name = b"job-state";
        out.extend_from_slice(&(state_name.len() as u16).to_be_bytes());
        out.extend_from_slice(state_name);
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&state.to_be_bytes());
        out.push(0x03);
        out
    }

    #[test]
    fn c_abi_jobs_returns_the_open_job() {
        let empty = crate::message::encode_response(0, 1);
        let (port, _rx) = serve_seq(vec![jobs_response(4, 5), empty]);
        let printer = open_on(port);
        let mut jobs = std::ptr::null_mut();
        let mut count = 0;
        assert_eq!(ipp_tool_jobs(printer, &mut jobs, &mut count), 0);
        assert_eq!(count, 1);
        let job = unsafe { &*jobs };
        assert_eq!(job.id, 4);
        assert_eq!(job.state, 5);
        assert_eq!(job.impressions, 0);
        assert_eq!(job.name.to_str().unwrap(), "");
        assert_eq!(job.reasons.to_str().unwrap(), "");
        ipp_tool_printer_free(printer);
    }

    #[test]
    fn c_abi_add_reads_the_make() {
        let (port, _rx) = serve(zebra_response());
        let uri = CString::new(format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        let mut printer = std::ptr::null_mut();
        assert_eq!(ipp_tool_add(uri.as_ptr(), &mut printer), 0);
        let make = unsafe { CStr::from_ptr(ipp_tool_make_model(printer)) };
        assert_eq!(make.to_str().unwrap(), "Zebra");
        ipp_tool_printer_free(printer);
    }

    #[test]
    fn a_missing_page_format_sends_nothing() {
        let uri = CString::new("ipp://127.0.0.1:9/ipp/print").unwrap();
        let mut printer = std::ptr::null_mut();
        assert_eq!(ipp_tool_printer_open(uri.as_ptr(), &mut printer), 0);
        let mut job_id = 11;
        assert_ne!(ipp_tool_print_test(printer, &mut job_id), 0);
        assert_eq!(job_id, 11);
        assert!(
            last_error().contains("no supported page format"),
            "{}",
            last_error()
        );
        ipp_tool_printer_free(printer);
    }

    #[test]
    fn python_embed_reads_the_make() {
        let (port, _rx) = serve(zebra_response());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = listener.local_addr().unwrap().port();
        drop(listener);
        let library = shared_library();
        assert!(library.is_file(), "missing {}", library.display());
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/test_embed.py");
        let mut command = python_command();
        let status = command
            .arg(&script)
            .env("IPP_TOOL_LIB", &library)
            .env("IPP_TOOL_URI", format!("ipp://127.0.0.1:{port}/ipp/print"))
            .env(
                "IPP_TOOL_DEAD_URI",
                format!("ipp://127.0.0.1:{dead}/ipp/print"),
            )
            .status()
            .expect("python3");
        assert!(status.success());
    }

    fn python_command() -> Command {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            let mut command = Command::new("arch");
            command.args(["-arm64", "/usr/bin/python3"]);
            command
        } else {
            Command::new("python3")
        }
    }

    fn shared_library() -> PathBuf {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = Command::new(cargo)
            .args(["build", "-p", "ipp-tool"])
            .status()
            .expect("cargo build -p ipp-tool");
        assert!(status.success());
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let filename = if cfg!(target_os = "macos") {
            "libipp_tool.dylib"
        } else if cfg!(target_os = "windows") {
            "ipp_tool.dll"
        } else {
            "libipp_tool.so"
        };
        target.join(profile).join(filename)
    }
}
