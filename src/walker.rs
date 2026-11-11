use crate::config::Config;
use crate::ports::fs::FileSystem;
use eyre::Result;
use log::{debug, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

pub struct FileWalker<F: FileSystem> {
    config: Arc<Config>,
    fs: Arc<F>,
}

impl<F: FileSystem> FileWalker<F> {
    pub fn new(config: Arc<Config>, fs: Arc<F>) -> Self {
        Self { config, fs }
    }

    pub fn collect_files(&self, root: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
        Ok(self.collect_files_counted(root, recursive)?.0)
    }

    /// Like `collect_files`, plus the count of entries the walk could not
    /// read: a descendant it could not open, or an eligible file whose
    /// metadata failed. Deliberate filters (exclusions, binary extensions,
    /// size) are not counted. `--check` fails on a non-zero count.
    pub fn collect_files_counted(&self, root: &Path, recursive: bool) -> Result<(Vec<PathBuf>, usize)> {
        debug!("Starting file collection from: {}", root.display());

        let mut files = Vec::new();
        let mut errors = 0;
        let walker = if recursive { WalkDir::new(root) } else { WalkDir::new(root).max_depth(1) };

        // Prune excluded paths before descending: an unreadable directory
        // under `node_modules/` is never scanned, so it is not a scan failure.
        for entry in walker.into_iter().filter_entry(|e| !self.is_excluded_path(e.path())) {
            match entry {
                Ok(entry) => {
                    let path = entry.path();

                    // Skip directories
                    if self.fs.is_dir(path) {
                        continue;
                    }

                    // Skip symlinks
                    if self.fs.is_symlink(path) {
                        debug!("Skipping symlink: {}", path.display());
                        continue;
                    }

                    match self.should_process_file(path) {
                        Some(true) => {
                            debug!("Adding file for processing: {}", path.display());
                            files.push(path.to_path_buf());
                        }
                        Some(false) => debug!("Filtering out file: {}", path.display()),
                        None => errors += 1,
                    }
                }
                Err(e) => {
                    warn!("Error accessing path during walk: {}", e);
                    errors += 1;
                }
            }
        }

        debug!("Collected {} files for processing, {} walk errors", files.len(), errors);
        Ok((files, errors))
    }

    /// `Some(true)` to process, `Some(false)` for a deliberate filter, `None`
    /// when the file's metadata could not be read.
    fn should_process_file(&self, path: &Path) -> Option<bool> {
        // Check if path matches exclusion patterns
        if self.is_excluded_path(path) {
            debug!("Path excluded by exclude-paths pattern: {}", path.display());
            return Some(false);
        }

        // Check if filename matches exclusion patterns
        if self.is_excluded_file(path) {
            debug!("File excluded by exclude-files pattern: {}", path.display());
            return Some(false);
        }

        // Check if file has binary extension
        if self.has_binary_extension(path) {
            debug!("File excluded by binary extension: {}", path.display());
            return Some(false);
        }

        // Check file size using FileSystem trait
        match self.fs.metadata(path) {
            Ok(metadata) => {
                if metadata.len > self.config.processing.max_file_size {
                    debug!("File too large ({}): {}", metadata.len, path.display());
                    return Some(false);
                }
            }
            Err(_) => {
                warn!("Could not read metadata for: {}", path.display());
                return None;
            }
        }

        Some(true)
    }

    fn is_excluded_path(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.config.exclude_paths {
            if let Ok(glob_pattern) = glob::Pattern::new(pattern)
                && glob_pattern.matches(&path_str)
            {
                return true;
            }

            // Also check if any parent directory matches the pattern
            for ancestor in path.ancestors() {
                let ancestor_str = ancestor.to_string_lossy();
                if let Ok(glob_pattern) = glob::Pattern::new(pattern)
                    && glob_pattern.matches(&ancestor_str)
                {
                    return true;
                }

                // Special handling for patterns like ".git/**"
                if pattern.ends_with("/**") {
                    let base_pattern = &pattern[..pattern.len() - 3]; // Remove "/**"
                    if ancestor.file_name().is_some_and(|name| {
                        glob::Pattern::new(base_pattern)
                            .is_ok_and(|base_glob| base_glob.matches(&name.to_string_lossy()))
                    }) {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn is_excluded_file(&self, path: &Path) -> bool {
        if let Some(filename) = path.file_name() {
            let filename_str = filename.to_string_lossy();

            for pattern in &self.config.exclude_files {
                if let Ok(glob_pattern) = glob::Pattern::new(pattern)
                    && glob_pattern.matches(&filename_str)
                {
                    return true;
                }
            }
        }

        false
    }

    fn has_binary_extension(&self, path: &Path) -> bool {
        if let Some(filename) = path.file_name() {
            let filename_str = filename.to_string_lossy();

            for pattern in &self.config.exclude_binary_extensions {
                if let Ok(glob_pattern) = glob::Pattern::new(pattern)
                    && glob_pattern.matches(&filename_str)
                {
                    return true;
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::fs::RealFs;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_config() -> Arc<Config> {
        Arc::new(Config::default())
    }

    /// RealFs for everything except `metadata`, which always fails.
    struct NoMetaFs;

    impl FileSystem for NoMetaFs {
        fn read(&self, path: &Path) -> Result<Vec<u8>> {
            RealFs.read(path)
        }
        fn write(&self, path: &Path, content: &[u8]) -> Result<()> {
            RealFs.write(path, content)
        }
        fn metadata(&self, path: &Path) -> Result<crate::ports::fs::FsMetadata> {
            Err(eyre::eyre!("no metadata: {}", path.display()))
        }
        fn is_dir(&self, path: &Path) -> bool {
            RealFs.is_dir(path)
        }
        fn is_file(&self, path: &Path) -> bool {
            RealFs.is_file(path)
        }
        fn is_symlink(&self, path: &Path) -> bool {
            RealFs.is_symlink(path)
        }
        fn exists(&self, path: &Path) -> bool {
            RealFs.exists(path)
        }
    }

    #[test]
    fn test_metadata_failure_is_counted_not_filtered() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        fs::write(root.join("a.txt"), "content").unwrap();
        fs::write(root.join("b.txt"), "content").unwrap();
        fs::write(root.join("skip.png"), "content").unwrap();

        let walker = FileWalker::new(create_test_config(), Arc::new(NoMetaFs));
        let (files, errors) = walker.collect_files_counted(root, true).unwrap();
        assert!(files.is_empty());
        assert_eq!(
            errors, 2,
            "the two eligible files fail; the binary extension is a filter"
        );
    }

    #[test]
    fn test_clean_walk_counts_no_errors() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("a.txt"), "content").unwrap();
        let walker = FileWalker::new(create_test_config(), Arc::new(RealFs));
        let (files, errors) = walker.collect_files_counted(temp_dir.path(), true).unwrap();
        assert_eq!((files.len(), errors), (1, 0));
    }

    #[test]
    fn test_collect_files_recursive() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create test files
        fs::write(root.join("test.txt"), "content").unwrap();
        fs::create_dir(root.join("subdir")).unwrap();
        fs::write(root.join("subdir").join("nested.rs"), "content").unwrap();

        let config = create_test_config();
        let real_fs = Arc::new(RealFs);
        let walker = FileWalker::new(config, real_fs);

        let files = walker.collect_files(root, true).unwrap();
        assert_eq!(files.len(), 2);

        let filenames: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(filenames.contains(&"test.txt".to_string()));
        assert!(filenames.contains(&"nested.rs".to_string()));
    }

    #[test]
    fn test_collect_files_non_recursive() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create test files
        fs::write(root.join("test.txt"), "content").unwrap();
        fs::create_dir(root.join("subdir")).unwrap();
        fs::write(root.join("subdir").join("nested.rs"), "content").unwrap();

        let config = create_test_config();
        let real_fs = Arc::new(RealFs);
        let walker = FileWalker::new(config, real_fs);

        let files = walker.collect_files(root, false).unwrap();
        assert_eq!(files.len(), 1);

        let filename = files[0].file_name().unwrap().to_string_lossy();
        assert_eq!(filename, "test.txt");
    }

    #[test]
    fn test_binary_extension_filtering() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create test files
        fs::write(root.join("test.txt"), "content").unwrap();
        fs::write(root.join("binary.exe"), "binary").unwrap();
        fs::write(root.join("library.so"), "binary").unwrap();

        let config = create_test_config();
        let real_fs = Arc::new(RealFs);
        let walker = FileWalker::new(config, real_fs);

        let files = walker.collect_files(root, false).unwrap();
        assert_eq!(files.len(), 1);

        let filename = files[0].file_name().unwrap().to_string_lossy();
        assert_eq!(filename, "test.txt");
    }

    #[test]
    fn test_exclude_paths_filtering() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create test files
        fs::write(root.join("test.txt"), "content").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git").join("config"), "git config").unwrap();

        let config = create_test_config();
        let real_fs = Arc::new(RealFs);
        let walker = FileWalker::new(config, real_fs);

        let files = walker.collect_files(root, true).unwrap();
        assert_eq!(files.len(), 1);

        let filename = files[0].file_name().unwrap().to_string_lossy();
        assert_eq!(filename, "test.txt");
    }
}
