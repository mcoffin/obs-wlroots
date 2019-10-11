use std::ffi;
use std::io;

pub struct ShmFd<S: AsRef<str>> {
    fd: libc::c_int,
    path: S,
}

impl<S: AsRef<str>> ShmFd<S> {
    pub fn open(path: S, oflag: libc::c_int, mode: libc::mode_t) -> io::Result<Self> {
        Some(unsafe { open(path.as_ref(), oflag, mode) })
            .filter(|&status| status >= 0)
            .map(|status| Ok(ShmFd {
                fd: status,
                path: path,
            }))
            .unwrap_or_else(|| Err(io::Error::last_os_error()))
    }

    #[inline(always)]
    pub fn as_raw(&self) -> libc::c_int {
        self.fd
    }

    #[inline(always)]
    fn path_ref(&self) -> &str {
        self.path.as_ref()
    }
}

impl<S: AsRef<str>> Drop for ShmFd<S> {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
            unlink(self.path_ref());
        };
    }
}

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
