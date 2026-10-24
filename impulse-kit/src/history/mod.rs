//! Command history shared by every terminal: the SQLite database of
//! commands run (`database`, the same file and schema as the Swift app's,
//! so both apps can share it) and readers for the shells' own history files
//! (`import`), so it can start with what the user already typed elsewhere.

pub mod database;
pub mod import;

pub use database::{CommandHistoryDatabase, HistoryEntry, HistoryFilter, HistoryHit};
