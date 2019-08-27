use std::ffi;

pub unsafe fn open<S: AsRef<str>>(name: S, oflag: libc::c_int, mode: libc::mode_t) -> libc::c_int {
    let name: &str = name.as_ref();
    let name = ffi::CString::new(name)
        .expect("invalid utf8 string");
    libc::shm_open(name.as_ptr(), oflag, mode)
}

pub unsafe fn unlink<S: AsRef<str>>(name: S) {
    let name: &str = name.as_ref();
    let name = ffi::CString::new(name)
        .expect("invalid utf8 string");
    libc::shm_unlink(name.as_ptr());
}
