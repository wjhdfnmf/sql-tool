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
