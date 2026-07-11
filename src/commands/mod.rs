//! Command modules grouped by product domain.
//!
//! Call sites should use domain paths:
//! `commands::memory::show`, `commands::capture::copy`, `commands::remote::serve`,
//! `commands::system::doctor`.

pub mod capture;
pub mod memory;
pub mod remote;
pub mod system;
