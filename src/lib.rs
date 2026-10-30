#![forbid(unsafe_code)]
//! molt: reclaim disk space on Linux, safely.

pub mod clean;
pub mod config;
pub mod processes;
pub mod rules;
pub mod safe_delete;
pub mod scan;
pub mod startup;
pub mod system;
