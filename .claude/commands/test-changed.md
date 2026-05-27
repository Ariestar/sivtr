Run the full quality gate on only the files changed since branching from main.

Steps:
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. Report results: how many tests passed, any warnings, any failures
