//! A minimal stand-in for the `webbrowser` crate, used only so links in the
//! window open in the default browser. The published crate links shlwapi,
//! which the self-contained GNU toolchain on this computer does not ship.

use std::io;

#[cfg(windows)]
pub fn open(url: &str) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut core::ffi::c_void,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> isize;
    }
    let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect() };
    let op = wide("open");
    let file = wide(url);
    // SW_SHOWNORMAL = 1. A result above 32 means success.
    let r = unsafe { ShellExecuteW(core::ptr::null_mut(), op.as_ptr(), file.as_ptr(), core::ptr::null(), core::ptr::null(), 1) };
    if r > 32 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, format!("could not open {url}")))
    }
}

#[cfg(not(windows))]
pub fn open(url: &str) -> io::Result<()> {
    let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(cmd).arg(url).spawn().map(|_| ())
}
