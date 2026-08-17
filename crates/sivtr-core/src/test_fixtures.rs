//! Shared on-disk git fixtures for tests that need repo/worktree layouts.

use std::fs;
use std::path::Path;

/// Create a normal repo (`root/.git` dir).
pub(crate) fn make_repo(root: &Path) {
    fs::create_dir_all(root.join(".git")).unwrap();
}

/// Create a linked worktree of `main` (mirrors `git worktree add`): the
/// worktree's `.git` is a `gitdir:` pointer to `<main>/.git/worktrees/<name>`,
/// whose `commondir` points back at the main `.git` dir.
pub(crate) fn make_worktree(main: &Path, wt: &Path, name: &str) {
    let gitdir = main.join(".git").join("worktrees").join(name);
    fs::create_dir_all(&gitdir).unwrap();
    fs::write(gitdir.join("commondir"), "../..").unwrap();
    fs::create_dir_all(wt).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
}
