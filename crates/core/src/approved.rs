//! Script files the user has chosen in a file dialog.
//!
//! The user interface never names a file to write on its own authority: it
//! can only save back to a file the user picked in a native Open or Save
//! dialog. That list is kept here, by the backend, and remembered between
//! runs, so that a query tab restored after a restart can still be saved to
//! the file it came from without asking again. The list is the backend's
//! own record of what the user chose; nothing the interface stores can add
//! a path to it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::Error;

/// Old entries drop off the front once the list is this long.
const MAX_FILES: usize = 300;

pub struct ApprovedFiles {
    path: PathBuf,
    /// Oldest first.
    files: Mutex<Vec<PathBuf>>,
}

impl ApprovedFiles {
    /// Reads the list kept at `path`. A missing or unreadable list is an empty one.
    pub fn load(path: PathBuf) -> Self {
        let files = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            files: Mutex::new(files),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<PathBuf>> {
        self.files.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether the user has chosen this exact file in a dialog, now or in an earlier run.
    pub fn contains(&self, file: &Path) -> bool {
        self.lock().iter().any(|known| known == file)
    }

    /// Records a file the user has just chosen. Failing to write the list to
    /// disk is reported, but the file counts as chosen for this run either way.
    pub fn add(&self, file: PathBuf) -> Result<(), Error> {
        let mut files = self.lock();
        files.retain(|known| known != &file);
        files.push(file);
        let extra = files.len().saturating_sub(MAX_FILES);
        files.drain(..extra);

        let storage = |e: &dyn std::fmt::Display| Error::Storage(e.to_string());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| storage(&e))?;
        }
        let bytes = serde_json::to_vec_pretty(&*files).map_err(|e| storage(&e))?;
        let tmp = self.path.with_extension("json.tmp");
        crate::files::write_private(&tmp, &bytes).map_err(|e| storage(&e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| storage(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sqltool-approved-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("scripts.json")
    }

    #[test]
    fn chosen_files_are_remembered_between_runs() {
        let path = temp("remembered");
        let first = ApprovedFiles::load(path.clone());
        assert!(!first.contains(Path::new("/work/report.sql")));
        first.add(PathBuf::from("/work/report.sql")).unwrap();
        first.add(PathBuf::from("/work/cleanup.sql")).unwrap();
        assert!(first.contains(Path::new("/work/report.sql")));

        // A new run of the app.
        let second = ApprovedFiles::load(path);
        assert!(second.contains(Path::new("/work/report.sql")));
        assert!(second.contains(Path::new("/work/cleanup.sql")));
        // Only the exact file, not its folder or a file beside it.
        assert!(!second.contains(Path::new("/work")));
        assert!(!second.contains(Path::new("/work/other.sql")));
        assert!(!second.contains(Path::new("/work/report.sql.bak")));
    }

    #[test]
    fn a_missing_or_damaged_list_is_empty_and_can_be_written_again() {
        let path = temp("damaged");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();
        let files = ApprovedFiles::load(path.clone());
        assert!(!files.contains(Path::new("/work/report.sql")));
        files.add(PathBuf::from("/work/report.sql")).unwrap();
        assert!(ApprovedFiles::load(path).contains(Path::new("/work/report.sql")));
    }

    #[test]
    fn the_list_is_bounded_and_a_file_chosen_again_moves_to_the_end() {
        let path = temp("bounded");
        let files = ApprovedFiles::load(path.clone());
        for i in 0..MAX_FILES + 20 {
            files.add(PathBuf::from(format!("/work/{i}.sql"))).unwrap();
        }
        assert!(!files.contains(Path::new("/work/0.sql")), "the oldest dropped off");
        assert!(files.contains(Path::new("/work/25.sql")));
        // Chosen again: kept as the newest, not duplicated.
        files.add(PathBuf::from("/work/25.sql")).unwrap();
        for i in 0..MAX_FILES - 1 {
            files.add(PathBuf::from(format!("/work/more-{i}.sql"))).unwrap();
        }
        assert!(files.contains(Path::new("/work/25.sql")));
        assert_eq!(ApprovedFiles::load(path).lock().len(), MAX_FILES);
    }
}
