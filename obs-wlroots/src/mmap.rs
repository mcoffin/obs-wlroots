use std::io;
use std::ptr;
use std::os::unix::io::RawFd;

pub struct MappedMemory {
    addr: *mut libc::c_void,
    size: libc::size_t,
}

impl MappedMemory {
    pub unsafe fn new(len: libc::size_t, prot: libc::c_int, flags: libc::c_int, fd: RawFd, offset: libc::off_t) -> io::Result<MappedMemory> {
        let addr = libc::mmap(ptr::null_mut(), len, prot, flags, fd, offset);
        if (addr as isize) == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(MappedMemory {
                addr: addr,
                size: len,
            })
        }
    }

    #[inline(always)]
    pub fn as_raw(&self) -> *mut libc::c_void {
        self.addr
    }
}

impl Drop for MappedMemory {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.addr, self.size);
        }
    }
}
