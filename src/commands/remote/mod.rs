pub mod mounts;
pub mod peer;
pub mod serve;
pub mod share;
pub mod workspace;

// `sivtr remote` command lives in mounts.rs so the domain module can be `commands::remote`.
pub use mounts::execute;
