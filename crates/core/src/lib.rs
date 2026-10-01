//! Core of UOC Journal: everything that does not need a GUI.
//!
//! * [`parse`]    – splits raw journal lines (`[time]  name: text`) and timestamps.
//! * [`classify`] – turns a speaker/text pair into a [`Channel`] using heuristics
//!   tuned for ClassicUO-based clients (Outlands in particular) plus user rules.
//! * [`pipeline`] – per-file state machine that feeds the classifier, tracks the
//!   logged-in character, fixes up name-label bursts, de-duplicates multi-client
//!   chat and produces [`Batch`]es.
//! * [`store`]    – compact, append-only in-memory journal (string arenas).
//! * [`query`]    – the search mini-language and per-pane incremental views.
//! * [`watcher`]  – background thread that discovers and tails journal files.

pub mod classify;
pub mod model;
pub mod names;
pub mod parse;
pub mod pipeline;
pub mod query;
pub mod store;
pub mod time;
pub mod watcher;

pub use model::{flags, Channel, ChannelSet, Entry};
pub use names::{Person, PersonKind};
pub use pipeline::{Batch, Pipeline, Session};
pub use query::{Filter, Query, View};
pub use store::Store;
