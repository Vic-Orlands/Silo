pub(crate) struct Locked {
    address: *const u8,
    length: usize,
    locked: bool,
}

impl Locked {
    pub(crate) fn new(bytes: &[u8]) -> Self {
        let locked = !bytes.is_empty() && lock(bytes.as_ptr(), bytes.len());
        Self {
            address: bytes.as_ptr(),
            length: bytes.len(),
            locked,
        }
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        if self.locked {
            unlock(self.address, self.length);
        }
    }
}

#[cfg(unix)]
fn lock(address: *const u8, length: usize) -> bool {
    unsafe { mlock(address.cast(), length) == 0 }
}

#[cfg(unix)]
fn unlock(address: *const u8, length: usize) {
    unsafe {
        let _ = munlock(address.cast(), length);
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn mlock(address: *const std::ffi::c_void, length: usize) -> i32;
    fn munlock(address: *const std::ffi::c_void, length: usize) -> i32;
}

#[cfg(windows)]
fn lock(address: *const u8, length: usize) -> bool {
    unsafe { VirtualLock(address.cast_mut().cast(), length) != 0 }
}

#[cfg(windows)]
fn unlock(address: *const u8, length: usize) {
    unsafe {
        let _ = VirtualUnlock(address.cast_mut().cast(), length);
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualLock(address: *mut std::ffi::c_void, length: usize) -> i32;
    fn VirtualUnlock(address: *mut std::ffi::c_void, length: usize) -> i32;
}

#[cfg(not(any(unix, windows)))]
fn lock(_address: *const u8, _length: usize) -> bool {
    false
}

#[cfg(not(any(unix, windows)))]
fn unlock(_address: *const u8, _length: usize) {}
