//! Core logic of File Organizer. This crate contains NO GUI code so that
//! every rule can be tested on its own.
//!
//! Stage 1: sort loose files at the top level of Downloads, Desktop and
//! Documents into `<Type>\<YYYY-MM>\` folders, with a journal and undo.
//! Later stages (duplicates, big/old files, renaming, background sorting)
//! are meant to plug in as new modules that reuse `safety`, `fsops`,
//! `journal` and `plan`.

pub mod config;
pub mod fsops;
pub mod journal;
pub mod paths;
pub mod plan;
pub mod safety;

pub use config::Settings;
pub use journal::{BatchReport, BatchSummary, Journal};
pub use paths::{RootKind, Roots};
pub use plan::{execute, scan, PlannedMove, Plan, ScanOptions};
