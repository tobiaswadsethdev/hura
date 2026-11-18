//! A Windows Job Object that ends every process put in it when `hurad` ends:
//! the forwards, and the updated binary a start hands over to.
//!
//! The Unix forward survives nothing it should not because a pipe closes
//! however a process dies, and a guard shell kills the forward when it does
//! (see `forward::GUARD`). Windows has the same guarantee built in: a job
//! created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` terminates its processes
//! when its last handle closes, and the handle here is only ever closed by
//! this process exiting, a crash or a `taskkill /F` included.

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

/// The job's handle, as an integer so it can sit in a static. Never closed.
fn job() -> Option<isize> {
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    *JOB.get_or_init(|| {
        // SAFETY: plain Win32 calls with no pointers kept beyond them; the
        // limit structure is zeroed and lives across the call that reads it.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            (ok != 0).then_some(job as isize)
        }
    })
}

/// Put the process behind `handle` in the job. A failure is reported rather
/// than fatal: the forward still works, it only outlives a `hurad` that dies
/// without stopping it.
pub fn adopt(handle: std::os::windows::io::RawHandle) -> Result<(), String> {
    let job = job().ok_or("could not create a job object")?;
    // SAFETY: `handle` is a live process handle owned by the caller's `Child`,
    // and the job handle is never closed.
    let ok = unsafe { AssignProcessToJobObject(job as HANDLE, handle as HANDLE) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
