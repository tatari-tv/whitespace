use crate::config::Config;
use crate::ports::fs::FileSystem;
use eyre::Result;
use log::{debug, warn};
use std::path::Path;
use std::sync::Arc;

pub struct WhitespaceProcessor<F: FileSystem> {
    config: Arc<Config>,
    fs: Arc<F>,
}

/// `error` for a binary file: skipped on purpose, never a failed scan.
pub const BINARY_SKIP: &str = "Binary file detected";
/// `error` for a file that is not UTF-8: skipped on purpose, never a failed scan.
pub const UTF8_SKIP: &str = "Invalid UTF-8 encoding";

#[derive(Debug, Clone)]
pub struct ProcessingResult {
    pub lines_modified: Vec<usize>,
    pub had_changes: bool,
    pub error: Option<String>,
}

impl ProcessingResult {
    /// True when the file could not be scanned (a read or write error), as
    /// opposed to the deliberate binary and invalid-UTF-8 skips. `--check`
    /// fails on these: a scan that missed a file cannot vouch for it.
    pub fn is_failure(&self) -> bool {
        matches!(self.error.as_deref(), Some(e) if e != BINARY_SKIP && e != UTF8_SKIP)
    }
}

impl<F: FileSystem> WhitespaceProcessor<F> {
    pub fn new(config: Arc<Config>, fs: Arc<F>) -> Self {
        Self { config, fs }
    }

    pub fn process_file(&self, path: &Path, dry_run: bool) -> Result<ProcessingResult> {
        debug!("Processing file: {}", path.display());

        // Read file content
        let content = match self.fs.read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                let error_msg = format!("Failed to read file: {}", e);
                warn!("{}: {}", error_msg, path.display());
                return Ok(ProcessingResult {
                    lines_modified: vec![],
                    had_changes: false,
                    error: Some(error_msg),
                });
            }
        };

        // Check if file is binary
        if self.is_binary_content(&content) {
            debug!("Skipping binary file: {}", path.display());
            return Ok(ProcessingResult {
                lines_modified: vec![],
                had_changes: false,
                error: Some(BINARY_SKIP.to_string()),
            });
        }

        // Convert to string
        let content_str = match String::from_utf8(content) {
            Ok(s) => s,
            Err(_) => {
                debug!("Skipping file with invalid UTF-8: {}", path.display());
                return Ok(ProcessingResult {
                    lines_modified: vec![],
                    had_changes: false,
                    error: Some(UTF8_SKIP.to_string()),
                });
            }
        };

        // Process content
        let (processed_content, modified_lines, _) = self.process_content(&content_str);
        let had_changes = !modified_lines.is_empty();

        // Write back if not dry run and there are changes
        if !dry_run && had_changes {
            if let Err(e) = self.fs.write(path, processed_content.as_bytes()) {
                let error_msg = format!("Failed to write file: {}", e);
                warn!("{}: {}", error_msg, path.display());
                return Ok(ProcessingResult {
                    lines_modified: modified_lines,
                    had_changes,
                    error: Some(error_msg),
                });
            }
            debug!("Wrote cleaned file: {}", path.display());
        }

        if had_changes {
            debug!("File processed: {} lines modified", modified_lines.len());
        }

        Ok(ProcessingResult {
            lines_modified: modified_lines,
            had_changes,
            error: None,
        })
    }

    pub fn process_content(&self, content: &str) -> (String, Vec<usize>, usize) {
        let mut processed_lines = Vec::new();
        let mut modified_line_numbers = Vec::new();
        let mut total_bytes_saved = 0;

        for (line_num, line) in content.lines().enumerate() {
            let original_len = line.len();
            let trimmed_line = line.trim_end();
            let trimmed_len = trimmed_line.len();

            if trimmed_len < original_len {
                // Line had trailing whitespace
                modified_line_numbers.push(line_num + 1); // 1-based line numbers
                total_bytes_saved += original_len - trimmed_len;
                debug!(
                    "Line {}: removed {} trailing bytes",
                    line_num + 1,
                    original_len - trimmed_len
                );
            }

            processed_lines.push(trimmed_line);
        }

        // Reconstruct content preserving original line endings
        let processed_content = if content.ends_with('\n') {
            format!("{}\n", processed_lines.join("\n"))
        } else {
            processed_lines.join("\n")
        };

        (processed_content, modified_line_numbers, total_bytes_saved)
    }

    fn is_binary_content(&self, content: &[u8]) -> bool {
        if !self.config.binary_detection.check_null_bytes {
            return false;
        }

        let sample_size = self.config.binary_detection.sample_size.min(content.len());
        let sample = &content[..sample_size];

        // Check for null bytes
        sample.contains(&0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::fs::{MemFs, RealFs};
    use std::fs;
    use tempfile::TempDir;

    fn create_test_config() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn test_process_content_trailing_spaces() {
        let config = create_test_config();
        let fs = Arc::new(MemFs::new());
        let processor = WhitespaceProcessor::new(config, fs);

        let content = "line1   \nline2\t\t\nline3\n";
        let (processed, modified_lines, bytes_saved) = processor.process_content(content);

        assert_eq!(processed, "line1\nline2\nline3\n");
        assert_eq!(modified_lines, vec![1, 2]);
        assert_eq!(bytes_saved, 5); // 3 spaces + 2 tabs
    }

    #[test]
    fn test_process_content_no_trailing_newline() {
        let config = create_test_config();
        let fs = Arc::new(MemFs::new());
        let processor = WhitespaceProcessor::new(config, fs);

        let content = "line1   \nline2\t\t";
        let (processed, modified_lines, bytes_saved) = processor.process_content(content);

        assert_eq!(processed, "line1\nline2");
        assert_eq!(modified_lines, vec![1, 2]);
        assert_eq!(bytes_saved, 5);
    }

    #[test]
    fn test_process_content_no_changes() {
        let config = create_test_config();
        let fs = Arc::new(MemFs::new());
        let processor = WhitespaceProcessor::new(config, fs);

        let content = "line1\nline2\nline3\n";
        let (processed, modified_lines, bytes_saved) = processor.process_content(content);

        assert_eq!(processed, content);
        assert_eq!(modified_lines.len(), 0);
        assert_eq!(bytes_saved, 0);
    }

    #[test]
    fn test_binary_detection() {
        let config = create_test_config();
        let fs = Arc::new(MemFs::new());
        let processor = WhitespaceProcessor::new(config, fs);

        let text_content = b"Hello, world!\n";
        let binary_content = b"Hello\0world\n";

        assert!(!processor.is_binary_content(text_content));
        assert!(processor.is_binary_content(binary_content));
    }

    #[test]
    fn test_process_file_with_memfs_dry_run() {
        let config = create_test_config();
        let original_content = b"line1   \nline2\t\t\n";
        let fs = Arc::new(MemFs::new().with_file("test.txt", original_content));
        let processor = WhitespaceProcessor::new(config, Arc::clone(&fs));

        let result = processor.process_file(Path::new("test.txt"), true).unwrap();

        assert!(result.had_changes);
        assert_eq!(result.lines_modified, vec![1, 2]);
        assert!(result.error.is_none());

        // File should not be modified in dry run
        let content = fs.get_content(Path::new("test.txt")).unwrap();
        assert_eq!(content, original_content);
    }

    #[test]
    fn test_process_file_with_memfs_actual_modification() {
        let config = create_test_config();
        let original_content = b"line1   \nline2\t\t\n";
        let fs = Arc::new(MemFs::new().with_file("test.txt", original_content));
        let processor = WhitespaceProcessor::new(config, Arc::clone(&fs));

        let result = processor.process_file(Path::new("test.txt"), false).unwrap();

        assert!(result.had_changes);
        assert_eq!(result.lines_modified, vec![1, 2]);
        assert!(result.error.is_none());

        // File should be modified
        let content = fs.get_content(Path::new("test.txt")).unwrap();
        assert_eq!(content, b"line1\nline2\n");
    }

    #[test]
    fn test_process_file_dry_run_with_real_fs() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");

        let original_content = "line1   \nline2\t\t\n";
        fs::write(&test_file, original_content).unwrap();

        let config = create_test_config();
        let real_fs = Arc::new(RealFs);
        let processor = WhitespaceProcessor::new(config, real_fs);

        let result = processor.process_file(&test_file, true).unwrap();

        assert!(result.had_changes);
        assert_eq!(result.lines_modified, vec![1, 2]);
        assert!(result.error.is_none());

        // File should not be modified in dry run
        let file_content = fs::read_to_string(&test_file).unwrap();
        assert_eq!(file_content, original_content);
    }

    #[test]
    fn test_process_file_actual_modification_with_real_fs() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");

        let original_content = "line1   \nline2\t\t\n";
        fs::write(&test_file, original_content).unwrap();

        let config = create_test_config();
        let real_fs = Arc::new(RealFs);
        let processor = WhitespaceProcessor::new(config, real_fs);

        let result = processor.process_file(&test_file, false).unwrap();

        assert!(result.had_changes);
        assert_eq!(result.lines_modified, vec![1, 2]);
        assert!(result.error.is_none());

        // File should be modified
        let file_content = fs::read_to_string(&test_file).unwrap();
        assert_eq!(file_content, "line1\nline2\n");
    }
}
