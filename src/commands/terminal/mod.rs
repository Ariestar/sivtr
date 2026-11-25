//! Terminal memory write surface: hooks, the pty proxy, session clear, and one-shot ingest.
//!
//! Read path is not here — use `workset` / `sivtr-core::query` (`terminal` source).

pub mod clear;
pub mod init;
pub mod pipe;
pub mod pty_proxy;
pub mod run;
