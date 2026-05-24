use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");

    if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
        println!(
            "cargo:rustc-env=SIVTR_BUILD_TIME_UNIX={}",
            duration.as_secs()
        );
    }

    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !hash.is_empty() {
                println!("cargo:rustc-env=SIVTR_GIT_HASH={hash}");
            }
        }
    }
}
