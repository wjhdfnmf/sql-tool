//! Core of the SQL tool: connection profiles, security policy, sessions.
//!
//! Nothing in this crate knows about the user interface.

pub mod approved;
pub mod chart;
pub mod ddl;
pub mod edit;
pub mod error;
pub mod export;
mod files;
pub mod guard;
pub mod history;
pub mod metadata;
pub mod policy;
pub mod probe;
pub mod profile;
pub mod profiles;
pub mod session;
pub mod settings;
pub mod slices;
pub mod split;
pub mod store;
pub mod table;
pub mod version;
pub mod workbench;

pub use approved::ApprovedFiles;
pub use ddl::ObjectKind;
pub use error::Error;
pub use files::decode_text;
pub use policy::PolicyError;
pub use profile::{CaSource, ConnectProfile, TlsMode};
pub use history::{History, HistoryEntry};
pub use metadata::{Charset, ColumnEntry, SchemaDefaults, SchemaObjects, TableDetails};
pub use profiles::{ProfileStore, SavedProfile, TlsSetting};
pub use settings::{Settings, SettingsStore};
pub use session::{Canceller, ResultSet, RunEvent, ServerInfo, Session, StreamStats};
pub use store::{ColumnInfo, ColumnKind, ColumnSource, ResultStore, RowBatch, StoreStatus};
pub use table::{ColumnSpec, ForeignKeySpec, IndexSpec, TableSpec};
pub use version::{Flavor, ServerVersion};
