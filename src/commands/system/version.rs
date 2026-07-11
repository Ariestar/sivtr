use anyhow::Result;
use chrono::{Local, TimeZone};

pub fn execute(verbose: bool) -> Result<()> {
    if !verbose {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let binary = std::env::current_exe()?;
    let cwd = std::env::current_dir()?;

    println!("sivtr {}", env!("CARGO_PKG_VERSION"));
    println!("binary: {}", binary.display());
    println!("cwd: {}", cwd.display());
    println!("profile: {}", build_profile());
    println!(
        "git commit: {}",
        option_env!("SIVTR_GIT_HASH").unwrap_or("unknown")
    );
    println!("build time: {}", build_time());

    if let Some(repo_root) = repo_root(&cwd) {
        println!("repo root: {}", repo_root.display());
        let debug_binary = repo_root.join(debug_binary_path());
        println!("local debug binary: {}", binary_status(&debug_binary));
        if debug_binary.exists() && !same_path(&binary, &debug_binary) {
            println!(
                "warning: running a different sivtr binary than the local debug build in this repo"
            );
        }
    }

    Ok(())
}

fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn build_time() -> String {
    option_env!("SIVTR_BUILD_TIME_UNIX")
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|seconds| Local.timestamp_opt(seconds, 0).single())
        .map(|timestamp| timestamp.format("%Y-%m-%dT%H:%M:%S%:z").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(windows)]
fn debug_binary_path() -> &'static str {
    "target/debug/sivtr.exe"
}

#[cfg(not(windows))]
fn debug_binary_path() -> &'static str {
    "target/debug/sivtr"
}

fn binary_status(path: &std::path::Path) -> String {
    if path.exists() {
        path.display().to_string()
    } else {
        format!("not found ({})", path.display())
    }
}

fn same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn repo_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    start
        .ancestors()
        .find(|path| path.join(".git").exists() && path.join("Cargo.toml").exists())
        .map(std::path::Path::to_path_buf)
}
