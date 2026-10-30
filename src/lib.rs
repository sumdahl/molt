#![forbid(unsafe_code)]
//! molt: reclaim disk space on Linux, safely.

pub mod analyze;
pub mod banner;
pub mod clean;
pub mod completions;
pub mod config;
pub mod history;
pub mod install;
pub mod installer;
pub mod menu;
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
