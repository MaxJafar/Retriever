//! OS copy/clone primitives, always exclusive and staged by the transfer journal.
use anyhow::{Context, Result};
use std::path::Path;

#[cfg(windows)]
pub fn copy(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CopyFileExW(
            source: *const u16,
            destination: *const u16,
            progress: *const std::ffi::c_void,
            data: *mut std::ffi::c_void,
            cancel: *mut i32,
            flags: u32,
        ) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // FAIL_IF_EXISTS. SMB can accept the compression hint; local copies use the
    // OS fast path and preserve alternate streams and filesystem attributes.
    let ok = unsafe {
        CopyFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0x10000001,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("Native copy failed");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn copy(source: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn copyfile(
            source: *const libc::c_char,
            destination: *const libc::c_char,
            state: *mut libc::c_void,
            flags: u32,
        ) -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // COPYFILE_ALL | EXCL | NOFOLLOW | CLONE (clone when supported, copy otherwise).
    let result = unsafe {
        copyfile(
            source.as_ptr(),
            destination.as_ptr(),
            std::ptr::null_mut(),
            0xf | (1 << 17) | (1 << 18) | (1 << 19) | (1 << 24),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("Native copy failed");
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn copy(source: &Path, destination: &Path) -> Result<()> {
    let mut input = std::fs::File::open(source)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    std::io::copy(&mut input, &mut output).context("Copy failed")?;
    output.set_permissions(input.metadata()?.permissions())?;
    output.set_times(std::fs::FileTimes::new().set_modified(input.metadata()?.modified()?))?;
    output.sync_all()?;
    Ok(())
}
