extern crate obs;
#[macro_use] extern crate wayland_client;

pub use obs::sys as obs_sys;

use std::ffi;
use std::ptr;

static mut MODULE_PTR: *mut obs_sys::obs_module = ptr::null_mut();

static mut SOURCE_INFO: Option<obs_sys::obs_source_info> = None;

const SOURCE_INFO_NAME: &'static [u8] = b"obs_wlroots\0";

pub mod source;

#[no_mangle]
pub extern "C" fn obs_module_load() -> bool {
    unsafe {
        SOURCE_INFO = Some(obs::source::empty_source());
        let source = SOURCE_INFO.as_mut().unwrap();
        source.id = ffi::CStr::from_bytes_with_nul_unchecked(SOURCE_INFO_NAME).as_ptr();
        source.type_ = obs_sys::obs_source_type_OBS_SOURCE_TYPE_INPUT;
        source.output_flags = obs_sys::OBS_SOURCE_VIDEO;
        source.get_name = Some(source::get_name);
        source.create = Some(source::create);
        source.destroy = Some(source::destroy);
        source.get_width = Some(source::get_width);
        source.get_height = Some(source::get_height);
    }
    unsafe {
        obs::source::register_source(SOURCE_INFO.as_ref().unwrap());
    }
    println!("libobs_wlroots loaded");
    true
}

pub extern "C" fn obs_module_unload() {
    println!("libobs_wlroots unloaded");
}

#[no_mangle]
pub unsafe extern "C" fn obs_module_set_pointer(module: *mut obs_sys::obs_module) {
    println!("MODULE_PTR: {:x}", MODULE_PTR as usize);
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
