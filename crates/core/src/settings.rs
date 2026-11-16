//! Application settings: what the user can change in the Settings screen.
//!
//! Kept as one small JSON file next to the saved connections. A missing or
//! unreadable file means the defaults; a setting outside its allowed range
//! is pulled back into it, so nothing read from disk is trusted as it is.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::Error;

pub const THEMES: [&str; 6] = ["system", "light", "dark", "midnight", "sepia", "contrast"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// One of [`THEMES`]. `system` follows the operating system's light or dark mode.
    pub theme: String,
    /// Size of everything in the window, in percent.
    pub zoom: u32,
    /// Spaces per indent in the SQL editor.
    pub editor_tab_size: u32,
    pub editor_word_wrap: bool,
    /// Offer keywords in upper case when completing.
    pub uppercase_keywords: bool,
    /// Bring back open query tabs when a connection is opened again.
    pub remember_tabs: bool,
    /// Rows kept per result for a new connection; `None` means no limit.
    pub default_row_limit: Option<u64>,
    /// Stop a statement that has run this long. 0 means never.
    pub query_timeout_secs: u32,
    /// Give up connecting after this long.
    pub connect_timeout_secs: u32,
    /// Send idle connections a ping this often so the server keeps them
    /// open. 0 means do not; a dropped connection is reopened when next used.
    pub keep_alive_secs: u32,
    /// Record executed statements in the history.
    pub keep_history: bool,
    /// Result data held in memory, across all open results, before more goes to disk.
    pub result_memory_mb: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            zoom: 100,
            editor_tab_size: 2,
            editor_word_wrap: false,
            uppercase_keywords: true,
            remember_tabs: true,
            default_row_limit: Some(10_000),
            query_timeout_secs: 0,
            connect_timeout_secs: 15,
            keep_alive_secs: 0,
            keep_history: true,
            result_memory_mb: 512,
        }
    }
}

impl Settings {
    /// The same settings with every value inside its allowed range.
    pub fn sanitized(mut self) -> Self {
        if !THEMES.contains(&self.theme.as_str()) {
            self.theme = "system".into();
        }
        self.zoom = self.zoom.clamp(70, 200);
        if ![2, 4, 8].contains(&self.editor_tab_size) {
            self.editor_tab_size = 2;
        }
        self.default_row_limit = self.default_row_limit.map(|rows| rows.clamp(1, 10_000_000));
        self.query_timeout_secs = self.query_timeout_secs.min(24 * 60 * 60);
        self.connect_timeout_secs = self.connect_timeout_secs.clamp(3, 300);
        if self.keep_alive_secs != 0 {
            self.keep_alive_secs = self.keep_alive_secs.clamp(30, 60 * 60);
        }
        self.result_memory_mb = self.result_memory_mb.clamp(64, 16 * 1024);
        self
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_secs.into())
    }

    pub fn query_timeout(&self) -> Option<Duration> {
        (self.query_timeout_secs > 0).then(|| Duration::from_secs(self.query_timeout_secs.into()))
    }

    pub fn keep_alive(&self) -> Option<Duration> {
        (self.keep_alive_secs > 0).then(|| Duration::from_secs(self.keep_alive_secs.into()))
    }
}

/// The settings file on disk.
pub struct SettingsStore {
    path: PathBuf,
}

fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The saved settings, or the defaults if there are none or the file cannot be read.
    pub fn load(&self) -> Settings {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .unwrap_or_default()
            .sanitized()
    }

    /// Saves the settings and returns them as stored (sanitized).
    pub fn save(&self, settings: Settings) -> Result<Settings, Error> {
        let settings = settings.sanitized();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(storage)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        crate::files::write_private(&tmp, &serde_json::to_vec_pretty(&settings).map_err(storage)?).map_err(storage)?;
        std::fs::rename(&tmp, &self.path).map_err(storage)?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> SettingsStore {
        let dir = std::env::temp_dir().join(format!("sqltool-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        SettingsStore::new(dir.join("settings.json"))
    }

    #[test]
    fn missing_or_broken_files_give_the_defaults() {
        let store = temp_store("defaults");
        assert_eq!(store.load(), Settings::default());
        std::fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        std::fs::write(&store.path, b"{ not json").unwrap();
        assert_eq!(store.load(), Settings::default());
    }

    #[test]
    fn settings_round_trip_and_unknown_fields_are_ignored() {
        let store = temp_store("roundtrip");
        let saved = store
            .save(Settings {
                theme: "midnight".into(),
                query_timeout_secs: 90,
                default_row_limit: None,
                ..Settings::default()
            })
            .unwrap();
        assert_eq!(store.load(), saved);
        assert_eq!(saved.query_timeout(), Some(Duration::from_secs(90)));

        // A file from a newer or older version: extra and missing fields are fine.
        std::fs::write(&store.path, br#"{"theme":"dark","somethingNew":true}"#).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.connect_timeout_secs, 15);
    }

    #[test]
    fn out_of_range_values_are_pulled_back() {
        let wild = Settings {
            theme: "<script>".into(),
            zoom: 5000,
            editor_tab_size: 3,
            default_row_limit: Some(0),
            query_timeout_secs: u32::MAX,
            connect_timeout_secs: 0,
            keep_alive_secs: 1,
            result_memory_mb: 1,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(wild.theme, "system");
        assert_eq!(wild.zoom, 200);
        assert_eq!(wild.editor_tab_size, 2);
        assert_eq!(wild.default_row_limit, Some(1));
        assert_eq!(wild.query_timeout_secs, 86_400);
        assert_eq!(wild.connect_timeout_secs, 3);
        assert_eq!(wild.keep_alive_secs, 30);
        assert_eq!(wild.result_memory_mb, 64);
        assert_eq!(Settings::default().keep_alive(), None);
        assert_eq!(Settings::default().query_timeout(), None);
    }
}
