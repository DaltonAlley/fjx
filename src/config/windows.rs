//! Windows system calls that restrict and verify the token file's access list.
//!
//! This is the crate's only unsafe code. Each call passes buffers that Rust
//! owns for the duration of the call, and every byte the system returns is
//! interpreted by the safe rules in `acl.rs`.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;

use super::acl;

type Handle = *mut c_void;

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER: i32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x0000_0001;
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_NEW: u32 = 1;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit_handle: i32,
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: i32,
        information: *mut c_void,
        length: u32,
        needed: *mut u32,
    ) -> i32;
    fn GetKernelObjectSecurity(
        handle: Handle,
        requested: u32,
        descriptor: *mut c_void,
        length: u32,
        needed: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        sharing: u32,
        security: *const SecurityAttributes,
        disposition: u32,
        attributes: u32,
        template: Handle,
    ) -> Handle;
}

/// Creates a new file that only the current user can open.
///
/// Its protected access list is supplied at creation, so inherited access never
/// exposes the file. No other handles can read or replace it while it is written.
pub(super) fn create_private(path: &Path) -> io::Result<File> {
    // Canonicalizing the existing parent gives CreateFileW a verbatim path,
    // including support for Unicode and paths longer than MAX_PATH.
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("missing filename"))?;
    let path = parent.canonicalize()?.join(name);
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    name.push(0);
    let user = current_user_sid()?;
    let descriptor = acl::private_descriptor(&user)
        .ok_or_else(|| io::Error::other("the current user SID is malformed"))?;
    let mut aligned = vec![0_u32; descriptor.len().div_ceil(4)];
    for (word, chunk) in aligned.iter_mut().zip(descriptor.chunks(4)) {
        let mut bytes = [0; 4];
        bytes[..chunk.len()].copy_from_slice(chunk);
        *word = u32::from_le_bytes(bytes);
    }
    let security = SecurityAttributes {
        length: u32::try_from(size_of::<SecurityAttributes>()).map_err(io::Error::other)?,
        descriptor: aligned.as_mut_ptr().cast(),
        inherit_handle: 0,
    };
    // SAFETY: `name` is NUL-terminated and `security` points to a DWORD-aligned,
    // self-relative descriptor. Both live through the call. CREATE_NEW prevents
    // opening an existing object, and the returned handle is not inheritable.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            &raw const security,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if raw == (-1_isize) as Handle {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW succeeded and transferred ownership of this handle.
    Ok(unsafe { File::from_raw_handle(raw) })
}

/// Checks that an open file is private to the current user.
pub(super) fn check_private(file: &File) -> io::Result<Result<(), &'static str>> {
    let user = current_user_sid()?;
    let descriptor = query(|buffer, length, needed| {
        // SAFETY: the handle is open with READ_CONTROL (part of GENERIC_READ),
        // and the buffer is writable for `length` bytes.
        unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                buffer,
                length,
                needed,
            )
        }
    })?;
    Ok(acl::check_private(&descriptor, &user))
}

fn current_user_sid() -> io::Result<Vec<u8>> {
    let mut raw = ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing,
    // and `raw` is a valid location for the token handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken succeeded, so `raw` is a token handle we own.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut base = 0;
    let information = query(|buffer, length, needed| {
        base = buffer as usize;
        // SAFETY: the token is open with TOKEN_QUERY, and the buffer is
        // writable for `length` bytes.
        unsafe { GetTokenInformation(token.as_raw_handle(), TOKEN_USER, buffer, length, needed) }
    })?;
    // TOKEN_USER starts with a pointer to the SID, which the system stores
    // inside the same buffer. Resolve it as an offset into the copied bytes.
    let pointer = information
        .get(..size_of::<usize>())
        .and_then(|bytes| bytes.try_into().ok())
        .map(usize::from_ne_bytes);
    pointer
        .and_then(|pointer| pointer.checked_sub(base))
        .and_then(|offset| information.get(offset..))
        .and_then(|sid| sid.get(..acl::sid_len(sid)?))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| io::Error::other("the current user SID is malformed"))
}

/// Calls a Windows "fill this buffer" function, growing the buffer until the
/// result fits, and returns the bytes it wrote.
fn query(mut call: impl FnMut(*mut c_void, u32, *mut u32) -> i32) -> io::Result<Vec<u8>> {
    // u64 words keep the buffer aligned for the structures Windows writes.
    let mut words = vec![0_u64; 32];
    let used = loop {
        let length = u32::try_from(words.len() * 8).map_err(io::Error::other)?;
        let mut needed = 0;
        if call(words.as_mut_ptr().cast(), length, &raw mut needed) != 0 {
            if needed > length {
                return Err(io::Error::other("invalid security information length"));
            }
            break usize::try_from(needed).map_err(io::Error::other)?;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER) || needed <= length {
            return Err(error);
        }
        words = vec![
            0;
            usize::try_from(needed)
                .map_err(io::Error::other)?
                .div_ceil(8)
        ];
    };
    Ok(words
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .take(used)
        .collect())
}
