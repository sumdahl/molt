#![forbid(unsafe_code)]
//! molt: reclaim disk space on Linux, safely.

pub mod analyze;
pub mod clean;
pub mod config;
pub mod installer;
pub mod optimize;
pub mod processes;
pub mod purge;
pub mod rules;
pub mod safe_delete;
pub mod scan;
pub mod startup;
pub mod status;
pub mod system;
mod terminal;
pub mod uninstall;
