//! The console a server on Windows was given and nobody is looking at.
//!
//! Task Scheduler starts a console program with a console window of its own,
//! which would sit open on the desktop for as long as the server runs. The
//! usual ways round it put something between the task and `hurad`
//! (`conhost --headless`, a hidden `cmd`, a script host), and stopping the
//! task then ends only that something: measured, the server went on holding
//! its port, so a restart started a second one that could not have it. So the
//! task starts `hurad` itself, and `hurad serve` puts the window away.
//!
//! It tells that console from a terminal by who else is attached: a console
//! made for this process alone has nobody else on it, where one in a
//! terminal has the shell that ran it. Only then does the server let go of
//! it, and what it would have printed goes to `hurad.log` in the state
//! directory instead, since nobody would see it there.
//!
//! Let go of rather than hidden: with Windows Terminal as the default terminal
//! application, the console's window is a stand-in and hiding it leaves the
//! terminal's own window open on the desktop. Measured on Windows 11. A
//! console with nobody attached closes, whichever kind of window it had, so
//! the window is there for the moment the server takes to start.

use std::path::Path;

use windows_sys::Win32::System::Console::{
    FreeConsole, GetConsoleProcessList, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};

/// Let go of this process's console and log to `log` instead, when the
/// console is its own. Returns whether it did.
pub fn put_away(log: &Path) -> bool {
    let mut attached = [0u32; 2];
    // SAFETY: the buffer is as long as the length passed.
    let count = unsafe { GetConsoleProcessList(attached.as_mut_ptr(), attached.len() as u32) };
    if count != 1 {
        return false;
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    else {
        return false;
    };
    use std::os::windows::io::IntoRawHandle as _;
    // Never closed: it is this process's stdout and stderr from here on, and
    // Rust's own `println!` asks for those handles on every write.
    let handle = file.into_raw_handle();
    // SAFETY: a valid file handle, owned for the rest of the process.
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
        SetStdHandle(STD_ERROR_HANDLE, handle);
        FreeConsole();
    }
    true
}
