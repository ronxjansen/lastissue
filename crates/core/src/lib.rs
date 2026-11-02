//! lastissue as a library: the on-disk model and storage, the repo
//! registry, and every operation front-ends share (the CLI and the GUI).
//!
//! No clap, no TTY, no printing: functions take plain structs and return
//! typed values or a [`error::Error`].

pub mod dates;
pub mod error;
pub mod identity;
pub mod index;
#[cfg(feature = "linear")]
pub mod linear;
pub mod model;
pub mod ops;
pub mod query;
pub mod registry;
pub mod search;
pub mod source;
pub mod store;
