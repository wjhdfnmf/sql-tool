//! Saved connection profiles. The file holds no secrets: passwords live in
//! the operating system keychain, keyed by profile id.

use std::path::PathBuf;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::profile::{CaSource, ConnectProfile, TlsMode};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsSetting {
    /// No encryption.
    Disabled,
    /// Encrypted, server certificate not checked. What most MySQL clients
    /// do by default.
    Unverified,
    VerifyCa,
    #[default]
    VerifyIdentity,
    /// Encrypted; the server must present the one certificate the user trusted.
    Pinned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedProfile {
    /// Empty for a profile that has not been saved yet.
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: Option<String>,
    pub tls: TlsSetting,
    pub ca_file: Option<PathBuf>,
    /// SHA-256 of the certificate trusted in pinned mode, as hex.
    pub pinned_sha256: Option<String>,
    /// Allow the `mysql_clear_password` plugin (LDAP and PAM accounts).
    pub cleartext_auth: bool,
    /// The user has acknowledged that cleartext auth without a verified
    /// server lets an impersonating server capture the password.
    pub cleartext_risk_accepted: bool,
    /// Keep the password in the OS keychain instead of asking each time.
    pub save_password: bool,
    /// Colour tag shown in the UI, for example to mark production.
    pub color: Option<String>,
    /// Refuse anything that is not a read, in the tool and on the server.
    pub read_only: bool,
    /// Ask before structure changes and before UPDATE or DELETE without WHERE.
    pub production: bool,
    /// Where this connection's scripts are kept. Open and Save As start in
    /// this folder. It only chooses where the file dialog begins; the user
    /// still picks the file.
    pub script_folder: Option<PathBuf>,
}

impl Default for SavedProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            host: String::new(),
            port: 3306,
            user: String::new(),
            database: None,
            tls: TlsSetting::default(),
            ca_file: None,
            pinned_sha256: None,
            cleartext_auth: false,
            cleartext_risk_accepted: false,
            save_password: false,
            color: None,
            read_only: false,
            production: false,
            script_folder: None,
        }
    }
}

impl SavedProfile {
    pub fn to_connect_profile(&self, password: SecretString) -> ConnectProfile {
        let mut profile = ConnectProfile::new(self.host.trim(), self.port, self.user.trim(), password);
        profile.database = self.database.clone().filter(|d| !d.trim().is_empty());
        profile.tls = match self.tls {
            TlsSetting::Disabled => TlsMode::Disabled,
            TlsSetting::Unverified => TlsMode::Required,
            TlsSetting::VerifyCa => TlsMode::VerifyCa,
            TlsSetting::VerifyIdentity => TlsMode::VerifyIdentity,
            TlsSetting::Pinned => TlsMode::Pinned,
        };
        profile.pinned_sha256 = self.pinned_sha256.as_deref().and_then(crate::probe::parse_fingerprint);
        profile.ca = self.ca_file.clone().map(CaSource::File);
        profile.allow_cleartext_auth = self.cleartext_auth;
        profile.accept_cleartext_on_unverified_transport = self.cleartext_risk_accepted;
        profile
    }
}

/// The profiles file on disk.
pub struct ProfileStore {
    path: PathBuf,
}

fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}

impl ProfileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Vec<SavedProfile>, Error> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                // Set the file aside: the app can then start with an empty
                // list, and saving a new connection cannot overwrite what
                // might still be recovered by hand.
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let aside = self.path.with_extension(format!("unreadable-{stamp}.json"));
                match std::fs::rename(&self.path, &aside) {
                    Ok(()) => Error::Storage(format!(
                        "the saved connections file was damaged ({e}). It has been kept as {} and the list \
                         starts empty. Saved passwords are still in the keychain",
                        aside.display()
                    )),
                    Err(_) => storage(e),
                }
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(storage(e)),
        }
    }

    fn save(&self, profiles: &[SavedProfile]) -> Result<(), Error> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(storage)?;
        }
        // Write to a temporary file and rename, so a crash mid-write cannot
        // leave a half-written profiles file.
        let tmp = self.path.with_extension("json.tmp");
        crate::files::write_private(&tmp, &serde_json::to_vec_pretty(profiles).map_err(storage)?).map_err(storage)?;
        std::fs::rename(&tmp, &self.path).map_err(storage)
    }

    /// Adds the profile, or replaces the one with the same id. A profile
    /// without an id gets a new one. Returns the stored profile.
    pub fn upsert(&self, mut profile: SavedProfile) -> Result<SavedProfile, Error> {
        let mut all = self.load()?;
        if profile.id.is_empty() {
            profile.id = uuid::Uuid::new_v4().simple().to_string();
        }
        match all.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile.clone(),
            None => all.push(profile.clone()),
        }
        self.save(&all)?;
        Ok(profile)
    }

    pub fn delete(&self, id: &str) -> Result<(), Error> {
        let mut all = self.load()?;
        all.retain(|p| p.id != id);
        self.save(&all)
    }

    pub fn get(&self, id: &str) -> Result<Option<SavedProfile>, Error> {
        Ok(self.load()?.into_iter().find(|p| p.id == id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> ProfileStore {
        let dir = std::env::temp_dir().join(format!("sqltool-profiles-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ProfileStore::new(dir.join("profiles.json"))
    }

    fn sample() -> SavedProfile {
        SavedProfile {
            name: "Reporting".into(),
            host: "db.example.test".into(),
            user: "ann".into(),
            ..SavedProfile::default()
        }
    }

    #[test]
    fn missing_file_means_no_profiles() {
        assert_eq!(temp_store("missing").load().unwrap(), Vec::new());
    }

    #[test]
    fn upsert_assigns_an_id_then_updates_in_place() {
        let store = temp_store("upsert");
        let saved = store.upsert(sample()).unwrap();
        assert_eq!(saved.id.len(), 32);

        let mut edited = saved.clone();
        edited.port = 3307;
        store.upsert(edited).unwrap();

        let all = store.load().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].port, 3307);
        assert_eq!(store.get(&saved.id).unwrap().unwrap().name, "Reporting");
    }

    #[test]
    fn delete_removes_only_that_profile() {
        let store = temp_store("delete");
        let a = store.upsert(sample()).unwrap();
        let b = store.upsert(sample()).unwrap();
        store.delete(&a.id).unwrap();
        let all = store.load().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, b.id);
    }

    #[test]
    fn the_file_never_contains_a_password_field() {
        let store = temp_store("nosecret");
        store.upsert(sample()).unwrap();
        let raw = std::fs::read_to_string(&store.path).unwrap().to_lowercase();
        assert!(raw.contains("savepassword"));
        assert!(!raw.contains("\"password\""));
    }

    #[test]
    fn the_script_folder_is_kept_and_older_files_without_one_still_load() {
        let store = temp_store("scripts");
        let saved = store
            .upsert(SavedProfile {
                script_folder: Some(PathBuf::from("/work/reports")),
                ..sample()
            })
            .unwrap();
        assert_eq!(store.get(&saved.id).unwrap().unwrap().script_folder, Some(PathBuf::from("/work/reports")));

        // A file written before the setting existed.
        std::fs::write(&store.path, br#"[{"id":"old","name":"Old","host":"db","port":3306,"user":"ann"}]"#).unwrap();
        let old = store.load().unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].script_folder, None);
    }

    #[test]
    fn a_damaged_file_is_set_aside_and_the_list_starts_empty() {
        let store = temp_store("damaged");
        store.upsert(sample()).unwrap();
        std::fs::write(&store.path, b"{ this is not json").unwrap();

        let error = store.load().unwrap_err().to_string();
        assert!(error.contains("damaged") && error.contains("unreadable-"), "{error}");
        // The app can carry on: the list is empty and can be saved to again.
        assert!(store.load().unwrap().is_empty());
        store.upsert(sample()).unwrap();
        assert_eq!(store.load().unwrap().len(), 1);
        // And the damaged file is still there to look at.
        let kept = std::fs::read_dir(store.path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains("unreadable-"));
        assert!(kept);
    }

    #[test]
    fn unverified_tls_with_cleartext_needs_the_acknowledgement() {
        let mut p = sample();
        p.tls = TlsSetting::Unverified;
        p.cleartext_auth = true;
        let blocked = p.to_connect_profile("x".to_string().into());
        assert!(crate::policy::check(&blocked).is_err());

        p.cleartext_risk_accepted = true;
        let allowed = p.to_connect_profile("x".to_string().into());
        assert!(crate::policy::check(&allowed).is_ok());
    }

    #[test]
    fn old_files_without_newer_fields_still_load() {
        let json = r#"[{"id":"abc","name":"Old","host":"h","user":"u"}]"#;
        let all: Vec<SavedProfile> = serde_json::from_str(json).unwrap();
        assert_eq!(all[0].port, 3306);
        assert_eq!(all[0].tls, TlsSetting::VerifyIdentity);
    }
}
