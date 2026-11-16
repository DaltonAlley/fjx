//! The Windows system RNG; no provider handle or persistent state is needed.
#![allow(unsafe_code)]

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut std::ffi::c_void,
        buffer: *mut u8,
        length: u32,
        flags: u32,
    ) -> i32;
}

pub(super) fn random_bytes() -> Result<[u8; 32], crate::error::Error> {
    let mut bytes = [0; 32];
    // SAFETY: the system-preferred RNG accepts a null algorithm with flag 2.
    // The writable buffer is valid for all 32 bytes for the duration of the call.
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 32, 2) };
    if status < 0 {
        return Err(crate::error::Error::local(
            "Windows random number generation failed",
        ));
    }
    Ok(bytes)
}
