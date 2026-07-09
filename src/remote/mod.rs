//! Remote device access — the client half of sivtr-to-sivtr peer reads.
//!
//! `config` holds the `remotes.toml` registry; `client` is the synchronous HTTP
//! client that calls a remote `sivtr serve` endpoint. The CLI's source loader
//! branches on a remote WorkRef origin and uses a [`client::RemoteClient`] to
//! fetch the same owned `WorkRecord`/`WorkPart` types local refs produce, so
//! downstream commands (`show`, `copy`, `filter`, …) are origin-agnostic.

pub mod client;
pub mod config;

pub use client::RemoteClient;
pub use config::{lookup, Remote, Remotes};
