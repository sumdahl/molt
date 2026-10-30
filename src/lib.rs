#![forbid(unsafe_code)]
//! molt: reclaim disk space on Linux, safely.

pub mod config;
pub mod rules;
pub mod safe_delete;
pub mod scan;
pub mod startup;
