//! Core of the SQL tool: connection profiles, security policy, sessions.
//!
//! Nothing in this crate knows about the user interface.

pub mod ddl;
pub mod error;
pub mod history;
pub mod metadata;
pub mod policy;
pub mod profile;
pub mod profiles;
pub mod session;
pub mod split;
pub mod store;
pub mod version;

pub use ddl::ObjectKind;
pub use error::Error;
pub use policy::PolicyError;
pub use profile::{CaSource, ConnectProfile, TlsMode};
pub use history::{History, HistoryEntry};
pub use metadata::{Charset, ColumnEntry, SchemaDefaults, SchemaObjects, TableDetails};
pub use profiles::{ProfileStore, SavedProfile, TlsSetting};
pub use session::{Canceller, ResultSet, RunEvent, ServerInfo, Session, StreamStats};
pub use store::{ColumnInfo, ColumnKind, ResultStore, RowBatch, StoreStatus};
pub use version::{Flavor, ServerVersion};
