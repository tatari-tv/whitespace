pub mod cli;
pub mod config;
pub mod engine;
pub mod ports;
pub mod processor;
pub mod walker;

pub use cli::Cli;
pub use config::{Config, RuntimeConfig};
pub use engine::{ParallelEngine, ProcessingResults, ProcessingSummary};
pub use ports::fs::{FileSystem, FsMetadata, MemFs, RealFs};
pub use processor::{ProcessingResult, WhitespaceProcessor};
pub use walker::FileWalker;

use colored::*;
use eyre::{Context, Result};
use log::info;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Format line numbers into compressed ranges (e.g., "1-3,5,7-10")
pub fn format_line_numbers(lines: &[usize]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut ranges = Vec::new();
    let mut start = lines[0];
    let mut end = lines[0];

    for &line in &lines[1..] {
        if line == end + 1 {
            end = line;
        } else {
            if start == end {
                ranges.push(start.to_string());
            } else {
                ranges.push(format!("{}-{}", start, end));
            }
            start = line;
            end = line;
        }
    }

    // Add the last range
    if start == end {
        ranges.push(start.to_string());
    } else {
        ranges.push(format!("{}-{}", start, end));
    }

    format!(" ({})", ranges.join(","))
}

/// Display processing results to the console.
/// Returns the number of files with changes.
pub fn display_results(file_results: &[(PathBuf, ProcessingResult)], is_dry_run: bool) -> usize {
    let mut files_with_changes = 0;

    for (file_path, result) in file_results {
        if result.had_changes && result.error.is_none() {
            let line_info = format_line_numbers(&result.lines_modified);
            println!("{}{}", file_path.display().to_string().blue(), line_info.dimmed());
            files_with_changes += 1;
        }
    }

    // Display summary with colors and icons
    if files_with_changes == 0 {
        println!("{}", "✅ No trailing whitespace found".green().bold());
    } else if is_dry_run {
        println!(
            "\n{} {} {}",
            "📋".cyan(),
            format!("{}", files_with_changes).cyan().bold(),
            "files NOT cleaned".yellow()
        );
    } else {
        println!(
            "\n{} {} {}",
            "🧹".green(),
            format!("{}", files_with_changes).cyan().bold(),
            "files cleaned".green().bold()
        );
    }

    files_with_changes
}

/// Process a single directory and return (files_with_changes, files_modified).
pub fn process_directory<F: FileSystem>(
    target_dir: &Path,
    runtime_config: &RuntimeConfig,
    fs: Arc<F>,
) -> Result<(usize, usize)> {
    info!("Processing directory: {}", target_dir.display());

    let file_config = Arc::new(runtime_config.file_config.clone());

    // Initialize file walker
    let walker = FileWalker::new(Arc::clone(&file_config), Arc::clone(&fs));

    // Collect files
    let files = walker
        .collect_files(target_dir, runtime_config.recursive)
        .with_context(|| format!("Failed to collect files from {}", target_dir.display()))?;

    if files.is_empty() {
        return Ok((0, 0));
    }

    info!("Found {} files to process in {}", files.len(), target_dir.display());

    // Initialize engine
    let engine =
        ParallelEngine::new(file_config, fs, runtime_config.threads).context("Failed to initialize parallel engine")?;

    // Process files and collect results for display
    let results = engine
        .process_files_with_results(files, runtime_config.dry_run)
        .with_context(|| format!("Failed to process files in {}", target_dir.display()))?;

    // Display results to console for this directory
    let files_with_changes = display_results(&results.file_results, runtime_config.dry_run);
    let actual_files_modified = if runtime_config.dry_run { 0 } else { files_with_changes };

    Ok((files_with_changes, actual_files_modified))
}

/// Main application entry point. Returns Ok(()) on success.
/// Returns the number of files that carry trailing whitespace.
///
/// Returned rather than only logged so `main` can turn it into an exit code:
/// `--check` has to fail a build, and until it did, `lint` steps built on this
/// tool could not fail at all. A plain run (or `--dry-run`) ignores the count.
pub fn run(runtime_config: &RuntimeConfig) -> Result<usize> {
    info!("Starting whitespace removal application");

    let fs = Arc::new(RealFs);

    info!("Target directories: {:?}", runtime_config.directories);
    info!("Recursive: {}", runtime_config.recursive);
    info!("Dry run: {}", runtime_config.dry_run);
    info!("Threads: {}", runtime_config.threads);

    let mut total_files_with_changes = 0;
    let mut total_files_modified = 0;
    let mut processed_dirs = 0;

    // Process each directory
    for target_dir in &runtime_config.directories {
        if !target_dir.exists() {
            eprintln!(
                "{} {} {}",
                "❌".red(),
                "Directory does not exist:".red(),
                target_dir.display().to_string().yellow()
            );
            continue;
        }

        if !target_dir.is_dir() {
            eprintln!(
                "{} {} {}",
                "❌".red(),
                "Not a directory:".red(),
                target_dir.display().to_string().yellow()
            );
            continue;
        }

        match process_directory(target_dir, runtime_config, Arc::clone(&fs)) {
            Ok((files_with_changes, files_modified)) => {
                total_files_with_changes += files_with_changes;
                total_files_modified += files_modified;
                processed_dirs += 1;
            }
            Err(e) => {
                eprintln!(
                    "{} {} {}: {}",
                    "⚠️".yellow(),
                    "Error processing".red(),
                    target_dir.display().to_string().yellow(),
                    e
                );
            }
        }
    }

    if processed_dirs == 0 {
        println!("{}", "No valid directories found to process".yellow());
        return Ok(0);
    }

    // Log summary information
    info!("Processing completed:");
    info!("  Directories processed: {}", processed_dirs);
    info!("  Files with changes: {}", total_files_with_changes);
    info!("  Files modified: {}", total_files_modified);

    Ok(total_files_with_changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_line_numbers_empty() {
        assert_eq!(format_line_numbers(&[]), "");
    }

    #[test]
    fn test_format_line_numbers_single() {
        assert_eq!(format_line_numbers(&[5]), " (5)");
    }

    #[test]
    fn test_format_line_numbers_range() {
        assert_eq!(format_line_numbers(&[1, 2, 3]), " (1-3)");
    }

    #[test]
    fn test_format_line_numbers_mixed() {
        assert_eq!(format_line_numbers(&[1, 2, 3, 5, 7, 8, 9]), " (1-3,5,7-9)");
    }

    #[test]
    fn test_format_line_numbers_separate() {
        assert_eq!(format_line_numbers(&[1, 3, 5]), " (1,3,5)");
    }
}
