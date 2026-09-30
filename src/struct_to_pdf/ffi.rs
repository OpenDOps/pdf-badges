use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;

use crate::struct_to_pdf::{
    index_template, parse_page, render_rows, Page, RenderError, SourceFormat, TemplateIndex,
};

pub struct PdfTemplate {
    page: Page,
    index: TemplateIndex,
    base_dir: PathBuf,
    field_names: Vec<CString>,
}

#[repr(C)]
pub struct PdfTemplateValue {
    pub field: *const c_char,
    pub value: *const c_char,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").unwrap());
}

fn set_message(message: &str) {
    let cleaned = message.replace('\0', " ");
    let text = CString::new(cleaned).unwrap_or_else(|_| CString::new("error").unwrap());
    LAST_ERROR.with(|slot| *slot.borrow_mut() = text);
}

fn set_error(err: &RenderError) {
    set_message(&err.message);
}

fn clear_error() {
    set_message("");
}

fn failure(message: &str) -> c_int {
    set_message(message);
    1
}

#[no_mangle]
pub extern "C" fn pdf_template_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn pdf_template_index_bytes(
    yaml: *const u8,
    len: usize,
    base_dir: *const c_char,
    out: *mut *mut PdfTemplate,
) -> c_int {
    match catch_unwind(AssertUnwindSafe(|| index_bytes(yaml, len, base_dir, out))) {
        Ok(code) => code,
        Err(_) => failure("panic"),
    }
}

fn index_bytes(
    yaml: *const u8,
    len: usize,
    base_dir: *const c_char,
    out: *mut *mut PdfTemplate,
) -> c_int {
    if out.is_null() {
        return failure("missing output");
    }
    unsafe {
        *out = ptr::null_mut();
    }
    if yaml.is_null() || base_dir.is_null() {
        return failure("missing input");
    }
    let bytes = unsafe { std::slice::from_raw_parts(yaml, len) };
    let source = match std::str::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => return failure("yaml is not utf-8"),
    };
    let dir = match unsafe { CStr::from_ptr(base_dir) }.to_str() {
        Ok(dir) => dir,
        Err(_) => return failure("base_dir is not utf-8"),
    };
    let page = match parse_page(source, SourceFormat::Yaml) {
        Ok(page) => page,
        Err(err) => {
            set_error(&err);
            return 1;
        }
    };
    let index = match index_template(&page) {
        Ok(index) => index,
        Err(err) => {
            set_error(&err);
            return 1;
        }
    };
    let field_names = index
        .fields
        .iter()
        .map(|field| {
            CString::new(field.name.as_str()).unwrap_or_else(|_| CString::new("").unwrap())
        })
        .collect();
    let template = Box::new(PdfTemplate {
        page,
        index,
        base_dir: PathBuf::from(dir),
        field_names,
    });
    unsafe {
        *out = Box::into_raw(template);
    }
    clear_error();
    0
}

#[no_mangle]
pub extern "C" fn pdf_template_field_count(template: *const PdfTemplate) -> usize {
    if template.is_null() {
        return 0;
    }
    unsafe { &*template }.index.fields.len()
}

#[no_mangle]
pub extern "C" fn pdf_template_field_name(
    template: *const PdfTemplate,
    index: usize,
) -> *const c_char {
    if template.is_null() {
        return ptr::null();
    }
    let template = unsafe { &*template };
    match template.field_names.get(index) {
        Some(field) => field.as_ptr(),
        None => ptr::null(),
    }
}

#[no_mangle]
pub extern "C" fn pdf_template_render(
    template: *const PdfTemplate,
    values: *const PdfTemplateValue,
    row_lengths: *const usize,
    row_count: usize,
    out_bytes: *mut *mut u8,
    out_len: *mut usize,
) -> c_int {
    match catch_unwind(AssertUnwindSafe(|| {
        render(template, values, row_lengths, row_count, out_bytes, out_len)
    })) {
        Ok(code) => code,
        Err(_) => failure("panic"),
    }
}

fn render(
    template: *const PdfTemplate,
    values: *const PdfTemplateValue,
    row_lengths: *const usize,
    row_count: usize,
    out_bytes: *mut *mut u8,
    out_len: *mut usize,
) -> c_int {
    if !out_bytes.is_null() {
        unsafe { *out_bytes = ptr::null_mut() };
    }
    if !out_len.is_null() {
        unsafe { *out_len = 0 };
    }
    if template.is_null() || out_bytes.is_null() || out_len.is_null() {
        return failure("missing input");
    }
    let rows = match rows_from_c(values, row_lengths, row_count) {
        Ok(rows) => rows,
        Err(err) => {
            set_error(&err);
            return 1;
        }
    };
    let template = unsafe { &*template };
    match render_rows(&template.page, &template.index, &rows, &template.base_dir) {
        Ok(bytes) => {
            let boxed = bytes.into_boxed_slice();
            let len = boxed.len();
            unsafe {
                *out_bytes = Box::into_raw(boxed) as *mut u8;
                *out_len = len;
            }
            clear_error();
            0
        }
        Err(err) => {
            set_error(&err);
            1
        }
    }
}

fn rows_from_c(
    values: *const PdfTemplateValue,
    row_lengths: *const usize,
    row_count: usize,
) -> Result<Vec<HashMap<String, String>>, RenderError> {
    if row_count == 0 {
        return Err(RenderError {
            context: "page".to_string(),
            message: "no rows".to_string(),
        });
    }
    if values.is_null() || row_lengths.is_null() {
        return Err(RenderError {
            context: "page".to_string(),
            message: "missing rows".to_string(),
        });
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut offset = 0usize;
    for row_index in 0..row_count {
        let width = unsafe { *row_lengths.add(row_index) };
        let mut row = HashMap::new();
        for slot in 0..width {
            let item = unsafe { &*values.add(offset + slot) };
            if item.field.is_null() || item.value.is_null() {
                return Err(RenderError {
                    context: "page".to_string(),
                    message: "missing field".to_string(),
                });
            }
            let field = unsafe { CStr::from_ptr(item.field) };
            let value = unsafe { CStr::from_ptr(item.value) };
            let field = field.to_str().map_err(|_| RenderError {
                context: "page".to_string(),
                message: "field is not utf-8".to_string(),
            })?;
            let value = value.to_str().map_err(|_| RenderError {
                context: "page".to_string(),
                message: "value is not utf-8".to_string(),
            })?;
            row.insert(field.to_string(), value.to_string());
        }
        offset += width;
        rows.push(row);
    }
    Ok(rows)
}

#[no_mangle]
pub extern "C" fn pdf_template_bytes_free(bytes: *mut u8, len: usize) {
    if bytes.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(std::slice::from_raw_parts_mut(bytes, len)));
    }
}

#[no_mangle]
pub extern "C" fn pdf_template_free(template: *mut PdfTemplate) {
    if template.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(template));
    }
}
