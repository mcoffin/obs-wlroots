use ::obs_sys as sys;

use std::mem;
use std::ptr;
use std::ffi;

/// Safe interface to `obs_properties_t`
pub struct Properties(*mut sys::obs_properties_t);

impl Properties {
    pub fn new() -> Properties {
        let ptr = unsafe {
            sys::obs_properties_create()
        };
        if ptr.is_null() {
            panic!("obs failed to create properties object");
        }
        Properties(ptr)
    }

    pub fn add_string_list<'a>(
        &'a mut self,
        name: &str,
        description: &str
    ) -> StringPropertyList<'a> {
        let name = ffi::CString::new(name)
            .expect("invalid utf8 string");
        let description = ffi::CString::new(description)
            .expect("invalid utf8 string");
        unsafe {
            let ptr = sys::obs_properties_add_list(
                self.0,
                mem::transmute(name.as_ptr()),
                mem::transmute(description.as_ptr()),
                sys::obs_combo_type_OBS_COMBO_TYPE_LIST,
                sys::obs_combo_format::OBS_COMBO_FORMAT_STRING
            );
            StringPropertyList::from_raw(ptr)
        }
    }

    /// Unsafely create a properties object from a raw pointer
    pub unsafe fn from_raw(ptr: *mut sys::obs_properties_t) -> Properties {
        Properties(ptr)
    }

    /// Leaks this properties to a raw pointer
    pub unsafe fn leak(&mut self) -> *mut sys::obs_properties_t {
        let ret = self.0;
        self.0 = ptr::null_mut();
        ret
    }

    /// Leaks this properties to a raw pointer, consuming the object
    pub unsafe fn into_raw(mut self) -> *mut sys::obs_properties_t {
        self.leak()
    }
}

impl Drop for Properties {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                sys::obs_properties_destroy(self.0);
            }
        }
    }
}

pub trait PropertyList<T: ?Sized> {
    fn add_item<S: AsRef<str>, A: AsRef<T>>(&mut self, name: S, item: A) -> usize;
    fn clear(&mut self);
}

pub struct StringPropertyList<'a>(&'a mut sys::obs_property_t);

impl<'a> StringPropertyList<'a> {
    unsafe fn from_raw(ptr: *mut sys::obs_property_t) -> StringPropertyList<'a> {
        let r: Option<&'a mut sys::obs_property_t> = mem::transmute(ptr);
        StringPropertyList(r.unwrap())
    }

    fn as_ptr(&mut self) -> *mut sys::obs_property_t {
        self.0 as *mut sys::obs_property_t
    }
}

impl<'a> PropertyList<str> for StringPropertyList<'a> {
    fn add_item<S: AsRef<str>, A: AsRef<str>>(&mut self, name: S, item: A) -> usize {
        let item: &str = item.as_ref();
        let c_item = ffi::CString::new(item)
            .expect("invalid utf8 string");
        let name: &str = name.as_ref();
        let c_name = ffi::CString::new(name)
            .expect("invalid utf8 string");
        unsafe {
            sys::obs_property_list_add_string(
                self.as_ptr(),
                mem::transmute(c_name.as_ptr()),
                mem::transmute(c_item.as_ptr())
            )
        }
    }

    fn clear(&mut self) {
        unsafe {
            sys::obs_property_list_clear(self.as_ptr())
        }
    }
}
