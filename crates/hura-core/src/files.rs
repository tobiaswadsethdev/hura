//! Reading a worktree's files, from outside it.
//!
//! The working copy is inside the sandbox, so every one of these is an exec.
//! That shapes the interface: directories are listed one at a time as a tree is
//! expanded rather than walked in one go, because a repository is tens of
//! thousands of files and a tree only ever shows the few that are open.
//!
//! **Reading is all there is.** The agent owns the working copy -- it is the
//! thing editing it, and two writers with no shared lock is how a file gets
//! half of each. What a person wants here is to see what the agent did, and to
//! say something about it, which is what the review is for.
//!
//! Paths are relative to the repository root and are checked here rather than
//! trusted: a client sending `../../etc/shadow` is asking to read the sandbox
//! outside the working copy, and while the sandbox is the isolation boundary
//! rather than this code, a file browser that will fetch anything on the
//! filesystem is not what this claims to be.
//!
//! [`fetch`] is the exception, on purpose. It is for taking a file *out* --
//! a screenshot the agent took, a report it wrote -- and those land wherever
//! the tool that made them chose, `/tmp` as often as the working copy. So it
//! takes an absolute path as well as a relative one. That is no wider than
//! what the same client already has: a terminal into the sandbox, where `cat`
//! reads anything. For a worktree session that is the server's own
//! filesystem, as the server's user -- which its terminal reaches just the
//! same. It still only reads.
//!
//! A folder comes out as a zip of everything under it but `.git`, made on the
//! server from what the sandbox sends (see [`crate::zip`]). The sandbox sends
//! each file as a line of base64 rather than as bytes behind a length: the
//! agent may still be writing one, and a file that grew between being
//! measured and being read would shift every byte after it into the wrong
//! file.

use serde::{Deserialize, Serialize};

use crate::backend::Backend;
use crate::seed::sh_quote;
use crate::session::{Session, State};

/// Sentinel a script prints when the working copy is not there at all.
///
/// Machine-readable, because it has to be told apart from anything the shell
/// might say on its own.
pub(crate) const NO_REPO: &str = "@@hura-no-repo@@";

/// What every pane says when a session has no working copy.
///
/// The tree, the file viewer and the git pane all ask the sandbox about the
/// same directory, and each of them used to answer with whatever `sh` said
/// about it -- `ls: cannot access '/sandbox/repo': No such file or directory`
/// in the tree, `sh: 1: cd: can't cd to /sandbox/repo` in git. That is the
/// shell answering a question nobody asked, in a path the reader has no reason
/// to have heard of.
///
/// Two sentences, because there are two ways to have no working copy and the
/// right thing to do differs: one is to wait, the other is to start again.
pub fn no_working_copy(session: &Session) -> String {
    match session.state {
        State::Creating | State::Seeding => format!(
            "`{}` is still being prepared; its files appear once the clone finishes",
            session.name
        ),
        _ => format!(
            "`{}` has no working copy: its clone never finished, so there is nothing here to \
             show. Create the session again to retry it.",
            session.name
        ),
    }
}

/// How much of a file is read. A viewer showing half a megabyte is already
/// showing more than anyone reads; the rest is a scroll bar lying about how
/// much there is.
const CAP: u64 = 512 * 1024;

/// How big an image is sent whole. A picture cut off at [`CAP`] is no picture
/// at all, so one is read to the end; past this it is a file to fetch some
/// other way, not one to look at on a phone.
const IMAGE_CAP: u64 = 16 * 1024 * 1024;

/// How big a file [`fetch`] hands over. Base64 in a JSON reply, through an
/// exec's stdout, so the whole of it is held in memory three times on the way;
/// past this it is a job for `docker cp`, not a button.
const FETCH_CAP: u64 = 32 * 1024 * 1024;

/// How many files a folder [`fetch`] takes. Every one is a few processes in
/// the sandbox, and a reply has to come back before the client gives up
/// waiting; a folder with more than this in it is a `node_modules`, not a set
/// of screenshots.
const FETCH_FILES: u64 = 1000;

/// A file, whole, for a client to save on its own machine.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    /// The file's own name, for the copy a client saves.
    pub name: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// Base64.
    pub data: String,
}

/// An image, whole, for a viewer to draw.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    /// `image/png` and the like, decided by the bytes rather than the name.
    pub mime: String,
    /// Base64.
    pub data: String,
}

/// One entry in a directory.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

/// A directory's contents, directories first and then by name -- which is how a
/// tree is read rather than how `ls` happens to sort.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dir {
    /// Relative to the repository root; empty for the root itself.
    pub path: String,
    pub entries: Vec<Entry>,
}

/// A file, as much of it as is worth showing.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileText {
    pub path: String,
    /// Empty when `binary`. Lossy where the bytes are not UTF-8, which is what
    /// a viewer wants: one replacement character beats refusing the file.
    pub text: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// Whether `text` is only the first [`CAP`] bytes of it.
    pub truncated: bool,
    /// Decided by a NUL in the first block, which is what `git` does and is
    /// right far more often than any extension list.
    pub binary: bool,
    /// The file as an image, when its first bytes say it is one a viewer can
    /// draw and it is no bigger than [`IMAGE_CAP`]. Absent from an older
    /// server, which is the same answer as a file that is not one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub image: Option<Picture>,
}

/// What kind of image the bytes start like, if any a viewer can draw. SVG is
/// left out: it is text, and shown as such.
fn image_kind(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Reject anything that is not a path inside the working copy.
///
/// Checked by component rather than by looking for `..` in the string: `a/../b`
/// is fine and `..%2f` is not a thing a path has in it, but a component that
/// *is* `..` is the one case that escapes.
pub fn clean(path: &str) -> Result<String, String> {
    let trimmed = path.trim().trim_start_matches('/');
    let mut parts = Vec::new();
    for part in trimmed.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err("that path leaves the working copy".into()),
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

/// A cleaned relative path, under the working copy of *this* session.
///
/// The root is the backend's, `/sandbox/repo`. The path was already checked
/// component by component by [`clean`], which is what makes joining it safe.
fn absolute(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{rel}")
    }
}

/// A shell's complaint, turned into a sentence when it is one of the two that
/// have a better wording, and kept as it is when it is not.
///
/// `No such file or directory` means the tree is showing something that has
/// since been deleted, which is worth saying in those words rather than in
/// `ls`'s. Anything else is a genuine surprise, and hiding what the sandbox
/// said about it would leave nothing to investigate.
fn gone_or(stderr: &str, gone: &str) -> String {
    let said = stderr.trim();
    if said.contains("No such file or directory") {
        return gone.to_string();
    }
    if said.is_empty() {
        return "the sandbox would not read it, and said nothing about why".to_string();
    }
    format!("the sandbox could not read it: {said}")
}

/// List one directory.
pub fn list(backend: &dyn Backend, session: &Session, path: &str) -> Result<Dir, String> {
    let rel = clean(path)?;
    let root = backend.paths(session).repo;
    // `-p` marks directories with a trailing slash, `-A` includes dotfiles but
    // not `.` and `..`. A filename with a newline in it would split into two
    // entries; git will not track one, and the cost of handling it is a format
    // busybox's `ls` does not have.
    let script = format!(
        "[ -d {root} ] || {{ printf '%s\\n' {mark}; exit 0; }}\nls -Ap -- {dir}",
        root = sh_quote(&root),
        mark = sh_quote(NO_REPO),
        dir = sh_quote(&absolute(&root, &rel)),
    );
    let out = backend
        .exec(session, &["sh", "-c", &script])
        .map_err(|e| e.to_string())?;
    // The root, rather than the directory asked for: a session whose clone
    // never happened has no tree at all, and saying that once is worth more
    // than saying `ls: cannot access` about every path in it.
    if out.trimmed() == NO_REPO {
        return Err(no_working_copy(session));
    }
    if !out.ok() {
        return Err(gone_or(&out.stderr, "that directory is not there any more"));
    }

    let mut entries: Vec<Entry> = out
        .trimmed()
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(|l| match l.strip_suffix('/') {
            Some(name) => Entry {
                name: name.to_string(),
                dir: true,
            },
            None => Entry {
                name: l.to_string(),
                dir: false,
            },
        })
        .collect();
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));

    Ok(Dir { path: rel, entries })
}

/// Read one file.
pub fn read(backend: &dyn Backend, session: &Session, path: &str) -> Result<FileText, String> {
    let rel = clean(path)?;
    let root = backend.paths(session).repo;
    if rel.is_empty() {
        return Err("that is the repository, not a file".into());
    }
    // Size first, then the capped head, base64 so the bytes survive a transport
    // that hands back a `String`: an exec's stdout is already lossy UTF-8, and a
    // source file with a stray byte in it would come back altered.
    let script = format!(
        "[ -d {root} ] || {{ printf '%s\\n' {mark}; exit 0; }}\np={path}; [ -f \"$p\" ] || {{ echo missing; exit 3; }}; wc -c < \"$p\"; head -c {CAP} \"$p\" | base64 | tr -d '\\n'",
        root = sh_quote(&root),
        mark = sh_quote(NO_REPO),
        path = sh_quote(&absolute(&root, &rel)),
    );
    let out = backend
        .exec(session, &["sh", "-c", &script])
        .map_err(|e| e.to_string())?;
    if out.trimmed() == NO_REPO {
        return Err(no_working_copy(session));
    }
    // Exit 3 is the script's own way of saying the file is gone, and it says it
    // on stdout -- so the old wording put an empty stderr after a colon.
    if out.exit_code == 3 || out.trimmed() == "missing" {
        return Err("that file is not there any more".into());
    }
    if !out.ok() {
        return Err(gone_or(&out.stderr, "that file is not there any more"));
    }

    let mut lines = out.trimmed().splitn(2, '\n');
    let bytes: u64 = lines
        .next()
        .and_then(|l| l.trim().parse().ok())
        .ok_or("the sandbox did not say how big that file is")?;
    let encoded = lines.next().unwrap_or("").trim();
    let raw = decode_base64(encoded).ok_or("the file came back malformed")?;

    let binary = raw.contains(&0);
    let image = match image_kind(&raw) {
        Some(mime) if bytes <= CAP => Some(Picture {
            mime: mime.into(),
            data: encoded.to_string(),
        }),
        Some(mime) if bytes <= IMAGE_CAP => Some(Picture {
            mime: mime.into(),
            data: whole(backend, session, &absolute(&root, &rel))?,
        }),
        _ => None,
    };
    Ok(FileText {
        path: rel,
        text: if binary {
            String::new()
        } else {
            String::from_utf8_lossy(&raw).into_owned()
        },
        bytes,
        truncated: bytes > CAP,
        binary: binary || image.is_some(),
        image,
    })
}

/// Where a path [`fetch`] is given points in the sandbox: an absolute one is
/// taken as it is, and a relative one is under the working copy, the way the
/// tree names it.
fn locate(root: &str, path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.starts_with('/') {
        let rel = clean(path).map_err(|_| "that path has a `..` in it".to_string())?;
        if rel.is_empty() {
            return Err("that is the whole sandbox, not a file".into());
        }
        Ok(format!("/{rel}"))
    } else {
        let rel = clean(path)?;
        if rel.is_empty() {
            return Err("that is the repository, not a file".into());
        }
        Ok(absolute(root, &rel))
    }
}

/// One file, whole, from anywhere in the sandbox, for a client to save; or a
/// folder, as a zip of the files in it.
pub fn fetch(backend: &dyn Backend, session: &Session, path: &str) -> Result<Fetched, String> {
    let full = locate(&backend.paths(session).repo, path)?;
    // Every refusal is decided in the sandbox, in the one exec, and said on
    // stdout with an exit code of its own -- the size as well, so a file or a
    // folder too big to send is not read just to find that out.
    //
    // A folder is walked twice: once to count and add up, and once to send.
    // `.git` is left out, and so is a name with a newline in it, which would
    // split into two lines of what is sent.
    let script = format!(
        "p={path}\n\
         [ -e \"$p\" ] || {{ echo missing; exit 3; }}\n\
         if [ -d \"$p\" ]; then\n\
         cd \"$p\" || exit 1\n\
         walk() {{ find . \\( -name .git -o -name '*\n*' \\) -prune -o -type f -print; }}\n\
         walk | {{\n\
         t=0; c=0\n\
         while IFS= read -r f; do\n\
         n=$(wc -c < \"$f\" | tr -d ' ') || continue\n\
         t=$((t + n)); c=$((c + 1))\n\
         [ \"$t\" -le {FETCH_CAP} ] || {{ echo \"$t\"; exit 6; }}\n\
         [ \"$c\" -le {FETCH_FILES} ] || {{ echo \"$c\"; exit 7; }}\n\
         done\n\
         [ \"$c\" -gt 0 ] || {{ echo empty; exit 8; }}\n\
         }} || exit $?\n\
         echo folder\n\
         walk | while IFS= read -r f; do\n\
         [ -f \"$f\" ] || continue\n\
         printf '%s\\n' \"${{f#./}}\"; base64 < \"$f\" | tr -d '\\n'; echo\n\
         done\n\
         exit 0\n\
         fi\n\
         [ -f \"$p\" ] || {{ echo special; exit 5; }}\n\
         n=$(wc -c < \"$p\" | tr -d ' ') || exit 1\n\
         [ \"$n\" -le {FETCH_CAP} ] || {{ echo \"$n\"; exit 6; }}\n\
         echo \"$n\"; base64 < \"$p\" | tr -d '\\n'",
        path = sh_quote(&full),
    );
    let out = backend
        .exec(session, &["sh", "-c", &script])
        .map_err(|e| e.to_string())?;
    let said = || out.trimmed().parse::<u64>().unwrap_or(0);
    match out.exit_code {
        0 => {}
        3 => return Err("that is not there any more".into()),
        5 => return Err("that is not a regular file".into()),
        6 => {
            return Err(format!(
                "that comes to {} MiB; a download takes up to {} MiB",
                said() >> 20,
                FETCH_CAP >> 20
            ));
        }
        7 => {
            return Err(format!(
                "that folder has more than {FETCH_FILES} files in it; a download takes up to {FETCH_FILES}"
            ));
        }
        8 => return Err("that folder has no files in it".into()),
        _ => return Err(gone_or(&out.stderr, "that is not there any more")),
    }
    let name = full.rsplit('/').next().unwrap_or_default().to_string();
    let mut lines = out.trimmed().split('\n');
    let first = lines.next().unwrap_or("").trim();
    if first == "folder" {
        return folder(&name, lines);
    }
    let bytes: u64 = first
        .parse()
        .map_err(|_| "the sandbox did not say how big that file is")?;
    Ok(Fetched {
        name,
        bytes,
        data: lines.next().unwrap_or("").trim().to_string(),
    })
}

/// A folder's files, as the sandbox sent them -- a path, then its bytes in
/// base64, and again -- packed into a zip under the folder's own name, so it
/// unpacks to a folder rather than into whatever directory it is opened in.
fn folder<'a>(name: &str, mut lines: impl Iterator<Item = &'a str>) -> Result<Fetched, String> {
    let mut members = Vec::new();
    while let Some(path) = lines.next() {
        if path.is_empty() {
            continue;
        }
        // Absent is an empty file that came last, whose empty line went with
        // the output's trailing whitespace.
        let data = decode_base64(lines.next().unwrap_or("").trim())
            .ok_or("the folder came back malformed")?;
        members.push(crate::zip::Member {
            path: format!("{name}/{path}"),
            data,
        });
    }
    let zip = crate::zip::write(&members, std::time::SystemTime::now());
    Ok(Fetched {
        name: format!("{name}.zip"),
        bytes: zip.len() as u64,
        data: encode_base64(&zip),
    })
}

/// A file read to the end, base64, for an image bigger than the first read.
fn whole(backend: &dyn Backend, session: &Session, path: &str) -> Result<String, String> {
    let script = format!(
        "head -c {IMAGE_CAP} {path} | base64 | tr -d '\\n'",
        path = sh_quote(path)
    );
    let out = backend
        .exec(session, &["sh", "-c", &script])
        .map_err(|e| e.to_string())?;
    if !out.ok() {
        return Err(gone_or(&out.stderr, "that file is not there any more"));
    }
    Ok(out.trimmed().to_string())
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64, without a dependency for it.
///
/// The encoder is usually the sandbox's `base64`, and this is the other half.
/// Sixty lines of crate for one function that has to exist anyway on the
/// client side of the terminal channel is not a trade worth making twice.
pub(crate) fn decode_base64(s: &str) -> Option<Vec<u8>> {
    // By table rather than by searching the alphabet: a folder taken out is
    // tens of megabytes of this.
    const VALUE: [u8; 256] = {
        let mut table = [0xFF; 256];
        let mut i = 0;
        while i < 64 {
            table[ALPHABET[i] as usize] = i as u8;
            i += 1;
        }
        table
    };
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for c in s.bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = match VALUE[c as usize] {
            0xFF => return None,
            v => v as u32,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// The encoding half, for what the server makes itself rather than reads out
/// of the sandbox: a folder's zip. Standard, with padding.
pub(crate) fn encode_base64(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len().div_ceil(3) * 4);
    for chunk in raw.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A component that *is* `..` escapes; one that merely contains dots does
    /// not, and a viewer that refused `..config` would be wrong.
    #[test]
    fn a_path_cannot_leave_the_working_copy() {
        assert!(clean("../../etc/shadow").is_err());
        assert!(clean("src/../../..").is_err());
        assert_eq!(clean("src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(clean("/src//main.rs").unwrap(), "src/main.rs");
        assert_eq!(clean("./src/./x").unwrap(), "src/x");
        assert_eq!(clean("..config").unwrap(), "..config");
        assert_eq!(clean("").unwrap(), "");
    }

    /// The root is the repository itself, not a path under it.
    #[test]
    fn the_empty_path_is_the_repository_root() {
        let root = crate::session::REPO_PATH;
        assert_eq!(absolute(root, ""), root);
        assert_eq!(absolute(root, "a/b"), format!("{root}/a/b"));
        // And the same rule wherever the working copy is.
        assert_eq!(absolute("/srv/worktrees/x", "a"), "/srv/worktrees/x/a");
    }

    /// An absolute path is anywhere in the sandbox, a relative one is in the
    /// working copy, and neither may be a directory's root or climb out.
    #[test]
    fn a_fetch_reads_absolute_paths_as_they_are() {
        let root = "/sandbox/repo";
        assert_eq!(locate(root, "/tmp/shot.png").unwrap(), "/tmp/shot.png");
        assert_eq!(locate(root, "//tmp/./a.md").unwrap(), "/tmp/a.md");
        assert_eq!(
            locate(root, "docs/a.md").unwrap(),
            "/sandbox/repo/docs/a.md"
        );
        assert!(locate(root, "/").is_err());
        assert!(locate(root, "").is_err());
        assert!(locate(root, "../x").is_err());
        assert!(locate(root, "/tmp/../etc/x").is_err());
    }

    /// By the bytes, not the name: a `.png` that is really text is text.
    #[test]
    fn images_are_told_by_their_first_bytes() {
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\n\0\0"), Some("image/png"));
        assert_eq!(image_kind(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(image_kind(b"GIF89a..."), Some("image/gif"));
        assert_eq!(image_kind(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_kind(b"RIFF\x10\0\0\0WAVEfmt "), None);
        assert_eq!(
            image_kind(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            None
        );
        assert_eq!(image_kind(b""), None);
    }

    #[test]
    fn base64_round_trips_what_the_sandbox_would_send() {
        // `printf 'hello\n' | base64` -- and a byte that is not UTF-8, which is
        // the case the encoding exists for.
        assert_eq!(decode_base64("aGVsbG8K").unwrap(), b"hello\n");
        assert_eq!(decode_base64("/w==").unwrap(), vec![0xff]);
        assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
        assert!(decode_base64("not base64!").is_none());
    }

    #[test]
    fn base64_encodes_what_it_decodes() {
        assert_eq!(encode_base64(b"hello\n"), "aGVsbG8K");
        assert_eq!(encode_base64(&[0xff]), "/w==");
        assert_eq!(encode_base64(b"ab"), "YWI=");
        assert_eq!(encode_base64(b""), "");
        let raw: Vec<u8> = (0..=255u8).cycle().take(1001).collect();
        assert_eq!(decode_base64(&encode_base64(&raw)).unwrap(), raw);
    }
}
