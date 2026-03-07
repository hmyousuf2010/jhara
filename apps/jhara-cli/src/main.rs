use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jhara_core::{scan, ScanConfig, ScanStats, ScanTree, NodeKind};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: jhara <path> [path2 ...]");
        eprintln!();
        eprintln!("Scans the given directories and prints a summary.");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  jhara ~/.cargo");
        eprintln!("  jhara ~/projects ~/.npm ~/.gradle");
        std::process::exit(1);
    }

    let roots: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();

    for root in &roots {
        if !root.exists() {
            eprintln!("Error: path does not exist: {}", root.display());
            std::process::exit(1);
        }
    }

    println!("Scanning {} root(s):", roots.len());
    for r in &roots {
        println!("  {}", r.display());
    }
    println!();

    let tree: Arc<Mutex<ScanTree>> = Arc::new(Mutex::new(ScanTree::with_capacity(500_000)));
    let tree_cb = Arc::clone(&tree);

    let config = ScanConfig {
        roots: roots.clone(),
        skip_list: HashSet::new(),
        stale_threshold_days: 90,
        prune_names: HashSet::new(),
    };

    let start = Instant::now();

    let (_handle, stats) = scan(config, move |batch| {
        tree_cb.lock().unwrap().insert_batch(batch);
    })
    .unwrap_or_else(|e| {
        eprintln!("Scan error: {}", e);
        std::process::exit(1);
    });

    let elapsed = start.elapsed();

    // Rollup sizes now that the scan is complete
    let mut tree = tree.lock().unwrap();
    tree.rollup();

    print_summary(&roots, &stats, elapsed, &tree);
}

fn print_summary(
    roots: &[PathBuf],
    stats: &ScanStats,
    elapsed: std::time::Duration,
    tree: &ScanTree,
) {
    let secs = elapsed.as_secs_f64();
    let rate = if secs > 0.0 {
        stats.total_entries as f64 / secs
    } else {
        0.0
    };

    println!("─────────────────────────────────────────");
    println!("Scan complete in {:.2}s", secs);
    println!("─────────────────────────────────────────");
    println!("  Entries scanned : {}", fmt_count(stats.total_entries));
    println!("  Throughput      : {:.0} entries/sec", rate);
    println!("  Physical bytes  : {}", fmt_bytes(stats.total_physical_bytes));
    println!("  Logical bytes   : {}", fmt_bytes(stats.total_logical_bytes));
    println!("  Deduped entries : {}", fmt_count(stats.deduped_entries));
    if stats.skipped_cloud_entries > 0 {
        println!("  Skipped (cloud) : {}", fmt_count(stats.skipped_cloud_entries));
    }
    if stats.error_count > 0 {
        println!("  Errors          : {}", stats.error_count);
    }
    println!("  Tree nodes      : {}", fmt_count(tree.len() as u64));
    println!(
        "  Tree memory     : {}",
        fmt_bytes(tree.approximate_heap_bytes() as u64)
    );
    println!();

    println!("Top 20 largest directories:");
    println!("─────────────────────────────────────────");

    // Collect all directory nodes, sort by physical size descending
    let mut dirs: Vec<(&std::path::Path, u64)> = tree
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::DirPre)
        .map(|n| (n.path.as_path(), n.physical_size))
        .collect();

    dirs.sort_by(|a, b| b.1.cmp(&a.1));

    for (i, (path, size)) in dirs.iter().take(20).enumerate() {
        println!("  {:>2}. {:>10}  {}", i + 1, fmt_bytes(*size), path.display());
    }

    println!();
    println!("Per-root breakdown:");
    println!("─────────────────────────────────────────");
    for root in roots {
        if let Some(size) = tree.physical_size(root) {
            println!("  {:>12}  {}", fmt_bytes(size), root.display());
        }
    }
}

fn fmt_bytes(bytes: u64) -> String {
    const KB: u64 = 1_024;
    const MB: u64 = KB * 1_024;
    const GB: u64 = MB * 1_024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn fmt_count(n: u64) -> String {
    // Insert thousands separator
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push('_');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
