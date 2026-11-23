// SPDX-License-Identifier: MIT
//! Example dynamic plugin — build as a cdylib and load with the manager.
//!
//! Build: cargo build --example echo_plugin
//! Output: target/debug/examples/libecho_plugin.{so,dll,dylib}

use libplugin::vtable::PluginVTable;
use std::os::raw::{c_char, c_int};

static NAME: &str = "echo_plugin\0";
static VERSION: &str = "1.0.0\0";

extern "C" fn name() -> *const c_char {
    NAME.as_ptr() as *const c_char
}

extern "C" fn version() -> *const c_char {
    VERSION.as_ptr() as *const c_char
}

extern "C" fn on_load(_config_json: *const c_char) -> c_int {
    0
}

extern "C" fn on_unload() -> c_int {
    0
}

/// Echo the message back, prefixed with "[echo]".
extern "C" fn on_message(
    msg_json: *const c_char,
    out_buf: *mut c_char,
    out_buf_len: usize,
) -> c_int {
    let msg = unsafe {
        if msg_json.is_null() {
            return -1;
        }
        std::ffi::CStr::from_ptr(msg_json)
            .to_str()
            .unwrap_or_default()
    };
    let response = format!("[echo] {}", msg);
    let bytes = response.as_bytes();
    if bytes.len() + 1 > out_buf_len {
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, out_buf, bytes.len());
        *out_buf.add(bytes.len()) = 0;
    }
    bytes.len() as c_int
}

extern "C" fn on_tick() -> c_int {
    0
}

extern "C" fn on_shutdown() -> c_int {
    0
}

/// Entry point resolved by `libloading` on the host side.
#[no_mangle]
pub extern "C" fn libplugin_entry() -> PluginVTable {
    PluginVTable {
        name,
        version,
        on_load,
        on_unload,
        on_message,
        on_tick,
        on_shutdown,
    }
}
