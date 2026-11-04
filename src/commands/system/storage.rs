//! `sivtr storage` command output.

use std::path::Path;

use anyhow::Result;
use sivtr_core::archive::db_path;
use sivtr_core::storage::{self, CacheEntry, StorageReport, VacuumReport};
use sivtr_core::workspace::home_dir;

use crate::cli::{
    StorageAction, StorageCommand, StoragePruneCacheArgs, StorageStatusArgs, StorageVacuumArgs,
};
use crate::output;

pub fn execute(command: &StorageCommand) -> Result<()> {
    match &command.action {
        StorageAction::Status(args) => status(args),
        StorageAction::Vacuum(args) => vacuum(args),
        StorageAction::PruneCache(args) => prune_cache(args),
    }
}

fn status(args: &StorageStatusArgs) -> Result<()> {
    let report = storage::report(&home_dir());
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    print_report(&report);
    Ok(())
}

fn print_report(report: &StorageReport) {
    println!("sivtr home: {}", report.home);
    println!("total: {}", format_bytes(report.total_bytes));
    println!();

    println!("archive");
    print_size("database", report.archive.database_bytes, None);
    if report.archive.wal_bytes > 0 {
        print_size("wal", report.archive.wal_bytes, None);
    }
    if report.archive.available {
        print_size(
            "free pages",
            report.archive.reclaimable_bytes,
            Some("estimated reclaimable"),
        );
    } else {
        println!("  {:<14} {:>10}  (archive unavailable)", "free pages", "-");
    }
    println!();

    println!("cache");
    print_size(
        "legacy",
        report.cache.legacy_bytes,
        Some(&format!("{} files, reclaimable", report.cache.legacy_files)),
    );
    print_size("bm25 indexes", report.cache.bm25_bytes, Some("rebuildable"));
    print_size("listing", report.cache.listing_bytes, Some("rebuildable"));
    print_size("unclassified", report.cache.other_bytes, None);
    println!();

    println!("{:<16} {:>10}", "sets", format_bytes(report.sets_bytes));
    println!(
        "{:<16} {:>10}",
        "workspaces",
        format_bytes(report.workspaces_bytes)
    );
    println!("{:<16} {:>10}", "other", format_bytes(report.other_bytes));
    if report.legacy_archive_bytes > 0 {
        println!(
            "{:<16} {:>10}  (stale copy at the home root)",
            "legacy archive",
            format_bytes(report.legacy_archive_bytes)
        );
    }
    println!();

    println!(
        "reclaimable now: {}",
        format_bytes(report.reclaimable_bytes)
    );
    if report.archive.reclaimable_bytes > 0 {
        println!("  sivtr storage vacuum");
    }
    if report.cache.legacy_bytes > 0 {
        println!("  sivtr storage prune-cache --apply");
    }
    println!();
    println!("provider-native session logs live outside this home and are not counted.");
}

fn print_size(label: &str, bytes: u64, note: Option<&str>) {
    match note {
        Some(note) => println!("  {label:<14} {:>10}  ({note})", format_bytes(bytes)),
        None => println!("  {:<14} {:>10}", label, format_bytes(bytes)),
    }
}

fn vacuum(args: &StorageVacuumArgs) -> Result<()> {
    let report = storage::vacuum(&db_path(), args.dry_run)?;
    if !report.exists {
        output::info("archive database does not exist; nothing to compact");
        return Ok(());
    }
    if !report.stats_available {
        output::warning("could not read archive page statistics; the database may be in use");
    }
    print_vacuum(&report);
    Ok(())
}

fn print_vacuum(report: &VacuumReport) {
    match report.after_bytes {
        Some(after) => {
            let freed = report.before_bytes.saturating_sub(after);
            output::success(format!(
                "compacted archive: {} -> {} (freed {})",
                format_bytes(report.before_bytes),
                format_bytes(after),
                format_bytes(freed)
            ));
        }
        _ => {
            println!("archive: {}", report.path);
            println!("database: {}", format_bytes(report.before_bytes));
            println!("reclaimable: {}", format_bytes(report.reclaimable_bytes));
            println!("dry run: run `sivtr storage vacuum` to compact");
        }
    }
}

fn prune_cache(args: &StoragePruneCacheArgs) -> Result<()> {
    let cache_dir = home_dir().join("cache");
    let mut entries = storage::legacy_cache_files(&cache_dir)?;
    entries.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.name.cmp(&right.name))
    });
    let total_bytes: u64 = entries.iter().map(|entry| entry.bytes).sum();
    let removed = if args.apply {
        Some(storage::remove_cache_files(&cache_dir, &entries)?)
    } else {
        None
    };

    if args.json {
        let (removed_files, freed_bytes) = removed.unwrap_or((0, 0));
        let report = serde_json::json!({
            "directory": cache_dir.display().to_string(),
            "files": &entries,
            "total_bytes": total_bytes,
            "applied": args.apply,
            "removed_files": removed_files,
            "freed_bytes": freed_bytes,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_cache(&cache_dir, &entries, total_bytes);
    }

    match removed {
        Some((0, _)) => {}
        Some((removed, freed)) => output::success(format!(
            "removed {removed} legacy cache files ({})",
            format_bytes(freed)
        )),
        None if !entries.is_empty() => output::hint("re-run with --apply to delete these files"),
        None => {}
    }
    Ok(())
}

fn print_cache(cache_dir: &Path, entries: &[CacheEntry], total_bytes: u64) {
    if entries.is_empty() {
        output::info(format!(
            "no legacy parsed-session caches in {}",
            cache_dir.display()
        ));
        return;
    }
    println!("cache: {}", cache_dir.display());
    println!(
        "{} legacy files, {}",
        entries.len(),
        format_bytes(total_bytes)
    );
    for entry in entries.iter().take(10) {
        println!("  {:<34} {:>10}", entry.name, format_bytes(entry.bytes));
    }
    if entries.len() > 10 {
        println!("  ... and {} more", entries.len() - 10);
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn byte_formatting_switches_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
