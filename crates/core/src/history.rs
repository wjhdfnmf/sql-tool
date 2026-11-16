//! Local history of executed statements, one JSON object per line.
//!
//! Statements that contain a password are never written out in full.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// The file is trimmed to its newer half when it grows past this.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Longer statements are cut off in the history (not when executed).
const MAX_SQL_CHARS: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub profile_name: String,
    pub sql: String,
    pub duration_ms: u64,
    pub ok: bool,
    /// Short outcome, for example `12 rows` or the error message.
    pub summary: String,
}

/// Returns the statement as it may be stored: unchanged, or with everything
/// after the first two words removed if it carries a password.
pub fn redact(sql: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    let flat: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    const MARKERS: [&str; 9] = [
        "identified by",
        "identified with",
        "identified via",
        "set password",
        "password(",
        "password (",
        "master_password",
        "source_password",
        "authentication_string",
    ];
    if MARKERS.iter().any(|m| flat.contains(m)) {
        let head: Vec<&str> = sql.split_whitespace().take(2).collect();
        return format!("{} \u{2026} [not saved: the statement contained a password]", head.join(" "));
    }
    if sql.chars().count() > MAX_SQL_CHARS {
        let cut: String = sql.chars().take(MAX_SQL_CHARS).collect();
        return format!("{cut}\u{2026}");
    }
    sql.to_string()
}

pub struct History {
    path: PathBuf,
    lock: Mutex<()>,
}

fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}

impl History {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn append(&self, mut entry: HistoryEntry) -> Result<(), Error> {
        entry.sql = redact(&entry.sql);
        let _guard = self.lock.lock().unwrap();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(storage)?;
        }
        if std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
            let text = std::fs::read_to_string(&self.path).map_err(storage)?;
            let lines: Vec<&str> = text.lines().collect();
            let newer = lines[lines.len() / 2..].join("\n") + "\n";
            std::fs::write(&self.path, newer).map_err(storage)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(storage)?;
        let mut line = serde_json::to_string(&entry).map_err(storage)?;
        line.push('\n');
        file.write_all(line.as_bytes()).map_err(storage)
    }

    /// Newest first. `search` matches anywhere in the statement, ignoring case.
    pub fn list(&self, search: Option<&str>, limit: usize) -> Result<Vec<HistoryEntry>, Error> {
        let _guard = self.lock.lock().unwrap();
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(storage(e)),
        };
        let needle = search.map(str::to_lowercase).filter(|s| !s.is_empty());
        Ok(text
            .lines()
            .rev()
            // A line that does not parse (a torn write) is skipped, not fatal.
            .filter_map(|line| serde_json::from_str::<HistoryEntry>(line).ok())
            .filter(|e| needle.as_ref().is_none_or(|n| e.sql.to_lowercase().contains(n)))
            .take(limit)
            .collect())
    }

    pub fn clear(&self) -> Result<(), Error> {
        let _guard = self.lock.lock().unwrap();
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(storage(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_history(name: &str) -> History {
        let dir = std::env::temp_dir().join(format!("sqltool-history-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        History::new(dir.join("history.jsonl"))
    }

    fn entry(sql: &str, at_ms: u64) -> HistoryEntry {
        HistoryEntry {
            at_ms,
            profile_name: "test".into(),
            sql: sql.into(),
            duration_ms: 5,
            ok: true,
            summary: "1 row".into(),
        }
    }

    #[test]
    fn statements_with_passwords_are_not_stored_in_full() {
        for sql in [
            "CREATE USER 'ann'@'%' IDENTIFIED BY 'hunter2'",
            "alter user ann identified   by 'hunter2'",
            "ALTER USER ann IDENTIFIED WITH caching_sha2_password BY 'hunter2'",
            "SET PASSWORD FOR ann = 'hunter2'",
            "SET PASSWORD = PASSWORD('hunter2')",
            "CHANGE MASTER TO MASTER_PASSWORD='hunter2'",
            "UPDATE mysql.user SET authentication_string = 'hunter2'",
        ] {
            let stored = redact(sql);
            assert!(!stored.contains("hunter2"), "leaked: {stored}");
            assert!(stored.contains("not saved"), "{stored}");
        }
    }

    #[test]
    fn ordinary_statements_are_stored_unchanged() {
        let sql = "SELECT password_hint, id FROM accounts WHERE id = 7";
        assert_eq!(redact(sql), sql);
    }

    #[test]
    fn very_long_statements_are_cut() {
        let sql = format!("SELECT '{}'", "x".repeat(MAX_SQL_CHARS * 2));
        assert_eq!(redact(&sql).chars().count(), MAX_SQL_CHARS + 1);
    }

    #[test]
    fn list_is_newest_first_and_searchable() {
        let history = temp_history("list");
        history.append(entry("SELECT 1", 1)).unwrap();
        history.append(entry("SELECT * FROM Orders", 2)).unwrap();
        history.append(entry("SELECT 3", 3)).unwrap();

        let all = history.list(None, 10).unwrap();
        assert_eq!(all.iter().map(|e| e.at_ms).collect::<Vec<_>>(), [3, 2, 1]);
        assert_eq!(history.list(None, 2).unwrap().len(), 2);

        let found = history.list(Some("orders"), 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sql, "SELECT * FROM Orders");
    }

    #[test]
    fn append_redacts_before_writing_to_disk() {
        let history = temp_history("redact");
        history.append(entry("CREATE USER a IDENTIFIED BY 'hunter2'", 1)).unwrap();
        let raw = std::fs::read_to_string(&history.path).unwrap();
        assert!(!raw.contains("hunter2"));
    }

    #[test]
    fn clear_empties_the_history_and_is_safe_to_repeat() {
        let history = temp_history("clear");
        history.append(entry("SELECT 1", 1)).unwrap();
        history.clear().unwrap();
        history.clear().unwrap();
        assert!(history.list(None, 10).unwrap().is_empty());
    }
}
