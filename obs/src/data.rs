use ::obs_sys as sys;

use std::borrow::Cow;
use std::ffi;
use std::mem;
use std::ops::{Deref, DerefMut};

pub struct Data(*mut sys::obs_data_t);

impl Data {
    pub unsafe fn from_raw(p: *mut sys::obs_data_t) -> Data {
        sys::obs_data_addref(p);
        Data(p)
    }
}

impl Deref for Data {
    type Target = sys::obs_data_t;

    fn deref(&self) -> &Self::Target {
        unsafe {
            self.0.as_ref().unwrap()
        }
    }
}

impl DerefMut for Data {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            self.0.as_mut().unwrap()
        }
    }
}

impl Drop for Data {
    fn drop(&mut self) {
        unsafe {
            sys::obs_data_release(self.0);
        }
    }
}

pub trait ObsData {
    fn get_int<S: AsRef<str>>(&self, key: S) -> i64;
    fn get_string<S: AsRef<str>>(&self, key: S) -> Option<&ffi::CStr>;
    fn get_double<S: AsRef<str>>(&self, key: S) -> f64;
    fn get_bool<S: AsRef<str>>(&self, key: S) -> bool;
    fn clear(&mut self);

    fn get_str<S: AsRef<str>>(&self, key: S) -> Option<Cow<str>> {
        self.get_string(key).map(|s| s.to_string_lossy())
    }
}

impl ObsData for sys::obs_data_t {
    fn get_int<S: AsRef<str>>(&self, key: S) -> i64 {
        let key: &str = key.as_ref();
        let c_key = ffi::CString::new(key)
            .expect("Invalid utf8 key");
        unsafe {
            sys::obs_data_get_int(mem::transmute(self as *const sys::obs_data_t), mem::transmute(c_key.as_bytes_with_nul().as_ptr()))
        }
    }
    fn get_string<S: AsRef<str>>(&self, key: S) -> Option<&ffi::CStr> {
        let key: &str = key.as_ref();
        let c_key = ffi::CString::new(key)
            .expect("Invalid utf8 key");
        let ptr = unsafe {
            sys::obs_data_get_string(mem::transmute(self as *const sys::obs_data), mem::transmute(c_key.as_bytes_with_nul().as_ptr()))
        };
        let ret = if ptr.is_null() {
            None
        } else {
            Some(ptr)
        };
        ret.map(|ptr| unsafe { ffi::CStr::from_ptr(ptr) })
    }
    fn get_double<S: AsRef<str>>(&self, key: S) -> f64 {
        let key: &str = key.as_ref();
        let c_key = ffi::CString::new(key)
            .expect("Invalid utf8 key");
        unsafe {
            sys::obs_data_get_double(
                mem::transmute(self as *const sys::obs_data_t),
                mem::transmute(c_key.as_bytes_with_nul().as_ptr())
            )
        }
    }
    fn get_bool<S: AsRef<str>>(&self, key: S) -> bool {
        let key: &str = key.as_ref();
        let c_key = ffi::CString::new(key)
            .expect("Invalid utf8 key");
        unsafe {
            sys::obs_data_get_bool(
                mem::transmute(self as *const sys::obs_data_t),
                mem::transmute(c_key.as_bytes_with_nul().as_ptr())
            )
        }
    }

    fn clear(&mut self) {
        unsafe {
            sys::obs_data_clear(self as *mut sys::obs_data);
        }
    }
}
