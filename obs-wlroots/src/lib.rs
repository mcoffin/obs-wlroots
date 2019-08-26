extern crate obs;
#[macro_use] extern crate wayland_client;

pub use obs::sys as obs_sys;

use std::ffi;
use std::ptr;

static mut MODULE_PTR: *mut obs_sys::obs_module = ptr::null_mut();

static mut SOURCE_INFO: Option<obs::source::SourceInfo> = None;

const SOURCE_INFO_NAME: &'static [u8] = b"obs_wlroots\0";

pub mod source;

#[no_mangle]
pub extern "C" fn obs_module_load() -> bool {
    unsafe {
        SOURCE_INFO = Some(obs::source::video_source_info::<source::WlrSource>());
        obs::source::register_source(SOURCE_INFO.as_ref().unwrap().as_raw());
    }
    println!("libobs_wlroots loaded");
    true
}

pub extern "C" fn obs_module_unload() {
    println!("libobs_wlroots unloaded");
}

#[no_mangle]
pub unsafe extern "C" fn obs_module_set_pointer(module: *mut obs_sys::obs_module) {
    MODULE_PTR = module;
}

#[no_mangle]
pub unsafe extern "C" fn obs_current_module() -> *mut obs_sys::obs_module {
    MODULE_PTR
}

#[no_mangle]
pub extern "C" fn obs_module_ver() -> u32 {
    obs::libobs_api_ver().into()
}
