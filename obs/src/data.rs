use ::obs_sys as sys;

use std::borrow::Cow;
use std::ffi;
use std::mem;

pub trait ObsData {
    fn get_string<S: AsRef<str>>(&self, key: S) -> Option<&ffi::CStr>;

    fn get_str<S: AsRef<str>>(&self, key: S) -> Option<Cow<str>> {
        self.get_string(key).map(|s| s.to_string_lossy())
    }
}

impl ObsData for sys::obs_data_t {
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
}
