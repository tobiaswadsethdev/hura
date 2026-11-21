//! Writing a zip file, for a folder taken out of a sandbox.
//!
//! Zip rather than a tarball because it is the one archive every place a
//! download lands opens without asking: Finder, Explorer, and the phone's Files
//! app, which will not open a `.tar.gz` at all.
//!
//! **Stored, not deflated.** What gets taken out is mostly screenshots, which
//! are compressed already, and a few pages of Markdown, which are small; a
//! deflater would be the larger part of this file for a saving nobody would
//! notice. And written here rather than pulled in as a crate, for the reason
//! the base64 decoder in `files.rs` is: a hundred lines of format that does not
//! change against a dependency tree that does. Without ZIP64, since a download
//! is capped far below the four gigabytes that needs.

use std::time::{SystemTime, UNIX_EPOCH};

/// One file in the archive: its path inside it, `/`-separated, and its bytes.
pub struct Member {
    pub path: String,
    pub data: Vec<u8>,
}

/// The archive, with every member dated `now`.
pub fn write(members: &[Member], now: SystemTime) -> Vec<u8> {
    let (time, date) = dos_time(now);
    let mut out = Vec::new();
    let mut central = Vec::new();
    for m in members {
        let offset = out.len() as u32;
        let crc = crc32(&m.data);
        let size = m.data.len() as u32;
        let name = m.path.as_bytes();

        // The local header, then the bytes.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        common(&mut out, time, date, crc, size, name.len());
        out.extend_from_slice(name);
        out.extend_from_slice(&m.data);

        // The same facts again in the directory at the end, which is what a
        // reader actually goes by, with where the local header is.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&(3u16 << 8 | 20).to_le_bytes()); // made by: Unix, 2.0
        common(&mut central, time, date, crc, size, name.len());
        central.extend_from_slice(&0u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        // `rw-r--r--`, in the high half where Unix readers look for it.
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let start = out.len() as u32;
    let count = members.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // the directory's disk
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

/// The fields a local header and a directory entry share, from the version
/// needed to extract through the extra field's length.
fn common(out: &mut Vec<u8>, time: u16, date: u16, crc: u32, size: u32, name: usize) {
    out.extend_from_slice(&20u16.to_le_bytes()); // needed: 2.0
    // Bit 11: the names are UTF-8. Without it a reader guesses at code page
    // 437, and a screenshot named in Swedish comes out as box drawings.
    out.extend_from_slice(&(1u16 << 11).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // stored
    out.extend_from_slice(&time.to_le_bytes());
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes()); // compressed
    out.extend_from_slice(&size.to_le_bytes()); // uncompressed
    out.extend_from_slice(&(name as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
}

/// CRC-32 as zip uses it: the reflected 0xEDB88320 polynomial.
fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[i] = c;
            i += 1;
        }
        table
    };
    !data.iter().fold(!0u32, |c, &b| {
        TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// MS-DOS time and date, in UTC: two-second resolution, years from 1980.
fn dos_time(t: SystemTime) -> (u16, u16) {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil(secs / 86_400);
    let s = secs % 86_400;
    let time = (s / 3600) << 11 | (s % 3600 / 60) << 5 | ((s % 60) / 2);
    let date = (y.clamp(1980, 2107) - 1980) << 9 | m << 5 | d;
    (time as u16, date as u16)
}

/// Days since 1970-01-01 as a year, month and day: Howard Hinnant's
/// `civil_from_days`, for the dates this can meet.
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_checksum_is_zips() {
        // The check value every CRC-32 implementation is tested against.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn dates_are_the_calendars() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(20_778), (2026, 11, 21));
        // 2026-11-21 10:40:16 UTC.
        let t = UNIX_EPOCH + Duration::from_secs(20_778 * 86_400 + 10 * 3600 + 40 * 60 + 16);
        let (time, date) = dos_time(t);
        assert_eq!(time, 10 << 11 | 40 << 5 | 8);
        assert_eq!(date, 46 << 9 | 11 << 5 | 21);
    }

    /// The system's own `unzip` is the judge: it checks every CRC and offset
    /// this file writes, and reads the names and bytes back.
    #[cfg(unix)]
    #[test]
    fn an_archive_is_one_unzip_reads() {
        let unzip = std::process::Command::new("unzip").arg("-v").output();
        if unzip.map(|o| !o.status.success()).unwrap_or(true) {
            return; // not on this machine; the format tests above still ran
        }
        let png: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let zip = write(
            &[
                Member {
                    path: "shots/a.png".into(),
                    data: png.clone(),
                },
                Member {
                    path: "shots/notes/räkna.md".into(),
                    data: b"# hej\n".to_vec(),
                },
            ],
            SystemTime::now(),
        );
        let dir = std::env::temp_dir().join(format!("hura-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.zip"), &zip).unwrap();
        let test = std::process::Command::new("unzip")
            .args(["-t", "-q", "x.zip"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            test.status.success(),
            "{}",
            String::from_utf8_lossy(&test.stdout)
        );
        let out = std::process::Command::new("unzip")
            .args(["-q", "x.zip", "-d", "out"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(std::fs::read(dir.join("out/shots/a.png")).unwrap(), png);
        assert_eq!(
            std::fs::read(dir.join("out/shots/notes/räkna.md")).unwrap(),
            b"# hej\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
