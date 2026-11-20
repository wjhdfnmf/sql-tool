//! Files the app keeps for one person: saved connections and the history.
//!
//! On Unix they are created readable by their owner only. Other accounts on
//! the same computer have no business reading which servers someone uses or
//! what they ran. On Windows the profile folder they live in is already
//! private to the account.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

fn private(options: &mut OpenOptions, path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.create(true).open(path)?;
    #[cfg(unix)]
    {
        // The mode above applies only to a file being created. One written
        // by an earlier version may be readable more widely.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// The text of a script file, or `None` if it is not text this app reads.
///
/// UTF-8 with or without a byte-order mark, and UTF-16 with one (what some
/// Windows tools write). The mark is never part of the text: a server would
/// take it for the start of the first statement and refuse the statement.
pub fn decode_text(bytes: &[u8]) -> Option<String> {
    fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
        let (pairs, rest) = bytes.as_chunks::<2>();
        if !rest.is_empty() {
            return None;
        }
        String::from_utf16(&pairs.iter().map(|pair| unit(*pair)).collect::<Vec<u16>>()).ok()
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).ok();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// Replaces the file's contents.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    private(OpenOptions::new().write(true).truncate(true), path)?.write_all(bytes)
}

/// Opens the file for adding to its end.
pub(crate) fn append_private(path: &Path) -> std::io::Result<File> {
    private(OpenOptions::new().append(true), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_files_are_read_whatever_mark_they_start_with() {
        assert_eq!(decode_text(b"SELECT 1").as_deref(), Some("SELECT 1"));
        assert_eq!(decode_text("SELECT 'é'".as_bytes()).as_deref(), Some("SELECT 'é'"));
        // The mark is dropped, so the first statement starts with its first letter.
        assert_eq!(decode_text(b"\xEF\xBB\xBFSELECT 1").as_deref(), Some("SELECT 1"));
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("SELECT 'é'".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode_text(&le).as_deref(), Some("SELECT 'é'"));
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("SELECT 'é'".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_text(&be).as_deref(), Some("SELECT 'é'"));
        assert_eq!(decode_text(b"").as_deref(), Some(""));
        // Not text: bytes that are no UTF-8, half a UTF-16 unit, half a surrogate pair.
        assert_eq!(decode_text(&[0x53, 0xE9, 0x4C]), None);
        assert_eq!(decode_text(&[0xFF, 0xFE, 0x53]), None);
        assert_eq!(decode_text(&[0xFF, 0xFE, 0x00, 0xD8]), None);
    }

    #[test]
    fn files_are_written_and_appended() {
        let dir = std::env::temp_dir().join(format!("sqltool-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("private.txt");
        write_private(&path, b"one\n").unwrap();
        append_private(&path).unwrap().write_all(b"two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        write_private(&path, b"three\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "three\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_readable_by_their_owner_only_even_if_they_were_not_before() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sqltool-files-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("was-public.txt");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"new").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let fresh = dir.join("fresh.txt");
        drop(append_private(&fresh).unwrap());
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
