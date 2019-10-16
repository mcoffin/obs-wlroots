extern crate libc;
extern crate obs_sys;

pub mod sys {
    pub use obs_sys::*;
}

pub mod data;
pub mod properties;
pub mod source;
pub mod gs;

pub use properties::Properties;

#[macro_export]
macro_rules! obs_declare_module {
    ($module_name:ident) => {
        pub mod $module_name {
            use std::ptr;

            static mut MODULE_PTR: *mut obs::sys::obs_module = ptr::null_mut();

            #[no_mangle]
            pub unsafe extern "C" fn obs_module_set_pointer(module: *mut obs::sys::obs_module) {
                MODULE_PTR = module;
            }

            #[no_mangle]
            pub unsafe extern "C" fn obs_current_module() -> *mut obs::sys::obs_module {
                MODULE_PTR
            }
        }
    };
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct SemanticVersion(u32);

impl SemanticVersion {
    pub const fn new(major: u8, minor: u8, patch: u8) -> SemanticVersion {
        let v = ((major as u32) << 24) | ((minor as u32) << 16) | (patch as u32);
        SemanticVersion(v)
    }

    #[inline(always)]
    pub fn major(&self) -> u8 {
        (self.0 >> 24) as u8
    }

    #[inline(always)]
    pub fn minor(&self) -> u8 {
        (self.0 >> 16) as u8
    }

    #[inline(always)]
    pub fn patch(&self) -> u8 {
        self.0 as u8
    }
}

impl Into<u32> for SemanticVersion {
    #[inline(always)]
    fn into(self) -> u32 {
        self.0
    }
}

pub const fn libobs_api_ver() -> SemanticVersion {
    SemanticVersion::new(
        sys::LIBOBS_API_MAJOR_VER as u8,
        sys::LIBOBS_API_MINOR_VER as u8,
        sys::LIBOBS_API_PATCH_VER as u8
    )
}
