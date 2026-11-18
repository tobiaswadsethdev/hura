//! Starting programs on this machine, the same way on every platform.
//!
//! Two things Windows needs said that Unix does not:
//!
//! * **No console windows.** A process without a console of its own, such as
//!   `hurad serve` under Task Scheduler or the desktop application, gets a new
//!   console window for every console program it starts, flashing up each
//!   time. `CREATE_NO_WINDOW` is the flag that says not to, and [`command`]
//!   sets it. Not for anything a person types into: a terminal is given its
//!   pseudo-console by the pty, and `hurad attach` hands over the console it
//!   was run in.
//! * **Windows' own `tar` and `curl`.** Both ship in `System32`, but a `PATH`
//!   with Git for Windows' `usr\bin` on it finds GNU tar first, which reads
//!   the `C:` of `C:\...` as a host to reach over ssh. [`system_tool`] names
//!   the inbox one.
//!
//! And one thing Windows lacks: a POSIX shell. hura's host-side scripts, a
//! worktree session's and the az credential's checks among them, are `sh`.
//! Git for Windows carries one, which Claude Code on Windows requires anyway,
//! and [`posix_shell`] finds it.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

/// `CREATE_NO_WINDOW`, from `winbase.h`.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A command for a program on this machine that nobody watches run.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// `name` as Windows ships it, `%SystemRoot%\System32\<name>.exe`, when that
/// exists; elsewhere, and failing that, the bare name for `PATH` to find.
pub fn system_tool(name: &str) -> PathBuf {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let inbox = PathBuf::from(root)
            .join("System32")
            .join(format!("{name}.exe"));
        if inbox.is_file() {
            return inbox;
        }
    }
    PathBuf::from(name)
}

/// A POSIX shell to run hura's scripts on this machine: `sh` on Unix, and Git
/// for Windows' `bash.exe` on Windows, or `None` there when it cannot be
/// found.
///
/// Never `bash` off the `PATH` on Windows: `System32\bash.exe` is WSL's
/// launcher, which runs the script in a Linux distribution instead, against
/// a different filesystem and different tools.
pub fn posix_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        git_bash(&|name| std::env::var_os(name), &|p| p.is_file())
    } else {
        Some(PathBuf::from("sh"))
    }
}

/// Git for Windows' `bash.exe`, in the order Claude Code itself looks:
/// `CLAUDE_CODE_GIT_BASH_PATH`, then beside the `git.exe` on `PATH`, then the
/// installer's default directories.
fn git_bash(
    env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    exists: &dyn Fn(&std::path::Path) -> bool,
) -> Option<PathBuf> {
    if let Some(set) = env("CLAUDE_CODE_GIT_BASH_PATH").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(set));
    }
    // `git.exe` is in `cmd\`, `bin\` or `mingw64\bin\` of the install; the
    // shell is `bin\bash.exe` of the same one.
    let from_git = env("PATH").into_iter().flat_map(|path| {
        std::env::split_paths(&path)
            .filter(|dir| exists(&dir.join("git.exe")))
            .filter_map(|dir| {
                let root = match dir.file_name()?.to_str()? {
                    "cmd" => dir.parent()?.to_path_buf(),
                    "bin" if dir.parent()?.file_name()? == "mingw64" => {
                        dir.parent()?.parent()?.to_path_buf()
                    }
                    "bin" => dir.parent()?.to_path_buf(),
                    _ => return None,
                };
                Some(root.join("bin").join("bash.exe"))
            })
            .collect::<Vec<_>>()
    });
    let defaults = [
        env("ProgramFiles").map(|p| PathBuf::from(p).join("Git")),
        env("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs").join("Git")),
    ]
    .into_iter()
    .flatten()
    .map(|root| root.join("bin").join("bash.exe"));
    from_git.chain(defaults).find(|bash| exists(bash))
}

/// The POSIX shell as a command, for a test that runs a script the way it
/// will be run: Git Bash on Windows, so those tests mean something there too.
#[cfg(test)]
pub(crate) fn sh() -> Command {
    command(posix_shell().expect("no POSIX shell; on Windows, install Git for Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn unix_finds_tools_on_its_path() {
        assert_eq!(system_tool("tar"), PathBuf::from("tar"));
    }

    #[test]
    #[cfg(windows)]
    fn windows_uses_its_own_tar_and_curl() {
        for tool in ["tar", "curl"] {
            let path = system_tool(tool);
            assert!(path.is_absolute(), "{tool}: {}", path.display());
            assert!(
                path.to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("system32"),
                "{tool}: {}",
                path.display()
            );
        }
    }

    fn env(
        vars: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    }

    /// The case this exists for: `C:\Windows\system32` comes first on a
    /// Windows `PATH` and has a `bash.exe` in it, WSL's.
    #[test]
    #[cfg(windows)]
    fn git_bash_is_found_beside_git_and_never_wsl_s() {
        let env = env(&[(
            "PATH",
            r"C:\Windows\system32;C:\Program Files\Git\cmd;C:\Program Files\nodejs",
        )]);
        let files = [
            r"C:\Windows\system32\bash.exe",
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files\Git\bin\bash.exe",
        ];
        let exists = |p: &std::path::Path| files.iter().any(|f| p == std::path::Path::new(f));
        assert_eq!(
            git_bash(&env, &exists),
            Some(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"))
        );
    }

    #[test]
    fn claude_code_s_own_setting_wins() {
        let env = env(&[("CLAUDE_CODE_GIT_BASH_PATH", r"D:\tools\bash.exe")]);
        assert_eq!(
            git_bash(&env, &|_| false),
            Some(PathBuf::from(r"D:\tools\bash.exe"))
        );
    }

    #[test]
    fn no_git_is_no_shell() {
        let env = env(&[("PATH", "")]);
        assert_eq!(git_bash(&env, &|_| false), None);
    }
}
