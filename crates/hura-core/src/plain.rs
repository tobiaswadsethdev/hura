//! Failures in words.
//!
//! Every message this crate builds ends up in front of somebody, in the
//! desktop window or on the phone, and the system's own wording -- `No such
//! file or directory (os error 2)`, `Connection refused (os error 61)` -- is
//! written for whoever is debugging the system, not for whoever is using it.
//! What a failure *means* here is usually narrower than what the OS can say,
//! so it is said here, once, for every place that would otherwise have passed
//! the system's sentence through.

use std::io::{self, ErrorKind};

/// An I/O failure, as a clause: what it means rather than the errno it was.
///
/// The kinds a connection or a process actually fails with each get their own
/// words. Anything else keeps the system's, less the `(os error N)` on the
/// end, which is the part nobody can act on.
pub fn io(e: &io::Error) -> String {
    match e.kind() {
        ErrorKind::NotFound => "it is not there".into(),
        ErrorKind::PermissionDenied => "permission was denied".into(),
        ErrorKind::ConnectionRefused => "nothing is listening there; is hurad running?".into(),
        ErrorKind::TimedOut | ErrorKind::WouldBlock => "it did not answer in time".into(),
        ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::BrokenPipe
        | ErrorKind::UnexpectedEof => "the connection was cut off".into(),
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable | ErrorKind::NetworkDown => {
            "this computer has no route to it; check the network".into()
        }
        ErrorKind::AddrNotAvailable => "that address is not available from here".into(),
        _ => {
            let text = e.to_string();
            match text.rfind(" (os error ") {
                Some(at) => text[..at].to_string(),
                None => text,
            }
        }
    }
}

/// A program that would not start: almost always one that is not installed,
/// which is worth saying in those words, since it is what to go and fix.
pub fn spawn(program: &str, e: &io::Error) -> String {
    match e.kind() {
        ErrorKind::NotFound => format!("`{program}` is not installed on the server"),
        _ => format!("could not run `{program}`: {}", io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_errno_does_not_survive() {
        let e = io::Error::from_raw_os_error(61);
        assert!(!io(&e).contains("os error"), "{}", io(&e));
        let odd = io::Error::other("the disk is on fire (os error 99)");
        assert_eq!(io(&odd), "the disk is on fire");
    }

    #[test]
    fn a_missing_program_is_said_to_be_missing() {
        let e = io::Error::from(ErrorKind::NotFound);
        assert_eq!(
            spawn("docker", &e),
            "`docker` is not installed on the server"
        );
    }
}
