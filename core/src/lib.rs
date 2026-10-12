//! Core logic of File Organizer. This crate contains NO GUI code so that
//! every rule can be tested on its own.
//!
//! Stage 1: sort loose files at the top level of Downloads, Desktop and
//! Documents into `<Type>\<YYYY-MM>\` folders, with a journal and undo.
//! Stage 2: duplicate finder (`dupes`), big / old file lists (`bigold`),
//! both walking sub-folders (`tree`); removal only ever sends files to the
//! Recycle Bin (`recycle`, `remove`) and is journaled.
//! Stage 3 (renaming, background sorting) will plug in as new modules.

pub mod bigold;
pub mod config;
pub mod dupes;
pub mod fsops;
pub mod journal;
pub mod paths;
pub mod plan;
pub mod recycle;
pub mod remove;
pub mod safety;
pub mod tree;

pub use config::Settings;
pub use journal::{BatchReport, BatchSummary, Journal};
pub use paths::{RootKind, Roots};
pub use plan::{execute, scan, Plan, PlannedMove, ScanOptions};
pub use recycle::{RecycleBin, Recycler};
pub use tree::{Progress, ScannedFile};
