use crate::cli::Cli;
use eyre::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// File extensions to process (empty = all text files)
    pub file_extensions: Vec<String>,

    /// Path patterns to exclude (glob patterns)
    pub exclude_paths: Vec<String>,

    /// Filename patterns to exclude (glob patterns)
    pub exclude_files: Vec<String>,

    /// Binary file extensions to exclude (fast pre-filter)
    pub exclude_binary_extensions: Vec<String>,

    /// Binary file detection settings
    pub binary_detection: BinaryDetection,

    /// Processing settings
    pub processing: ProcessingSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct BinaryDetection {
    /// Check for null bytes to detect binary files
    pub check_null_bytes: bool,

    /// Maximum bytes to read for binary detection
    pub sample_size: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ProcessingSettings {
    pub max_file_size: u64,
    #[serde(deserialize_with = "deserialize_threads")]
    pub threads: usize,
}

impl<'de> Deserialize<'de> for ProcessingSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "kebab-case")]
        enum Field {
            MaxFileSize,
            Threads,
        }

        struct ProcessingSettingsVisitor;

        impl<'de> Visitor<'de> for ProcessingSettingsVisitor {
            type Value = ProcessingSettings;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct ProcessingSettings")
            }

            fn visit_map<V>(self, mut map: V) -> Result<ProcessingSettings, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut max_file_size = None;
                let mut threads = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::MaxFileSize => {
                            if max_file_size.is_some() {
                                return Err(de::Error::duplicate_field("max-file-size"));
                            }
                            max_file_size = Some(map.next_value()?);
                        }
                        Field::Threads => {
                            if threads.is_some() {
                                return Err(de::Error::duplicate_field("threads"));
                            }
                            threads = Some(deserialize_threads_value(map.next_value()?)?);
                        }
                    }
                }

                let max_file_size = max_file_size.unwrap_or(100 * 1024 * 1024);
                let threads = threads.unwrap_or_else(num_cpus::get);

                Ok(ProcessingSettings { max_file_size, threads })
            }
        }

        const FIELDS: &[&str] = &["max-file-size", "threads"];
        deserializer.deserialize_struct("ProcessingSettings", FIELDS, ProcessingSettingsVisitor)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            file_extensions: vec![],
            exclude_paths: vec![
                // Version control
                ".git/**".to_string(),
                ".svn/**".to_string(),
                ".hg/**".to_string(),
                // Dependencies and virtual environments
                "node_modules/**".to_string(),
                ".venv/**".to_string(),
                "venv/**".to_string(),
                ".env/**".to_string(),
                "env/**".to_string(),
                "__pycache__/**".to_string(),
                ".tox/**".to_string(),
                ".pytest_cache/**".to_string(),
                // Build outputs
                "target/**".to_string(),
                "build/**".to_string(),
                "dist/**".to_string(),
                "out/**".to_string(),
                "bin/**".to_string(),
                "obj/**".to_string(),
                // IDE and editor files
                ".vscode/**".to_string(),
                ".idea/**".to_string(),
                ".vs/**".to_string(),
                "*.tmp/**".to_string(),
                // Package managers
                ".npm/**".to_string(),
                ".yarn/**".to_string(),
                ".pnpm-store/**".to_string(),
                "vendor/**".to_string(),
            ],
            exclude_files: vec![
                "*.min.js".to_string(),
                "*.min.css".to_string(),
                "*.bundle.*".to_string(),
                "*.lock".to_string(),
                "*.log".to_string(),
            ],
            exclude_binary_extensions: vec![
                // Executables and libraries
                "*.exe".to_string(),
                "*.dll".to_string(),
                "*.so".to_string(),
                "*.dylib".to_string(),
                "*.a".to_string(),
                "*.lib".to_string(),
                "*.bin".to_string(),
                "*.out".to_string(),
                // Archives
                "*.zip".to_string(),
                "*.tar".to_string(),
                "*.gz".to_string(),
                "*.bz2".to_string(),
                "*.xz".to_string(),
                "*.7z".to_string(),
                "*.rar".to_string(),
                // Images
                "*.jpg".to_string(),
                "*.jpeg".to_string(),
                "*.png".to_string(),
                "*.gif".to_string(),
                "*.bmp".to_string(),
                "*.ico".to_string(),
                "*.svg".to_string(),
                "*.webp".to_string(),
                // Audio/Video
                "*.mp3".to_string(),
                "*.mp4".to_string(),
                "*.avi".to_string(),
                "*.mov".to_string(),
                "*.wav".to_string(),
                "*.flac".to_string(),
                // Documents
                "*.pdf".to_string(),
                "*.doc".to_string(),
                "*.docx".to_string(),
                "*.xls".to_string(),
                "*.xlsx".to_string(),
                "*.ppt".to_string(),
                "*.pptx".to_string(),
                // Other binary formats
                "*.sqlite".to_string(),
                "*.db".to_string(),
                "*.dat".to_string(),
                "*.pyc".to_string(),
                "*.class".to_string(),
                "*.jar".to_string(),
            ],
            binary_detection: BinaryDetection::default(),
            processing: ProcessingSettings::default(),
        }
    }
}

impl Default for BinaryDetection {
    fn default() -> Self {
        Self {
            check_null_bytes: true,
            sample_size: 8192,
        }
    }
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            max_file_size: 100 * 1024 * 1024, // 100MB
            threads: num_cpus::get(),
        }
    }
}

use serde::de;

fn deserialize_threads_value<E>(value: serde_yaml::Value) -> Result<usize, E>
where
    E: de::Error,
{
    match value {
        serde_yaml::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                if u == 0 {
                    return Err(E::custom("threads must be greater than 0"));
                }
                Ok(u as usize)
            } else {
                Err(E::custom("threads must be a positive integer"))
            }
        }
        serde_yaml::Value::String(s) => match s.as_str() {
            "nproc" => Ok(num_cpus::get()),
            _ => Err(E::custom(format!(
                "invalid thread value: '{}', expected a positive integer or 'nproc'",
                s
            ))),
        },
        _ => Err(E::custom("threads must be a positive integer or the string 'nproc'")),
    }
}

/// XDG config dir, honoring `$XDG_CONFIG_HOME` and falling back to `$HOME/.config`.
fn xdg_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }
    dirs::home_dir().map(|h| h.join(".config"))
}

/// XDG data dir, honoring `$XDG_DATA_HOME` and falling back to `$HOME/.local/share`.
///
/// We deliberately do NOT use the `dirs` config/data helpers: those honor
/// `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` only on Linux. On macOS they resolve via system
/// APIs and return `~/Library/...`, ignoring the env vars. These helpers resolve to the
/// same XDG layout on every platform.
pub fn xdg_data_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }
    dirs::home_dir().map(|h| h.join(".local").join("share"))
}

impl Config {
    /// Load configuration with fallback chain
    pub fn load(config_path: Option<&PathBuf>) -> Result<Self> {
        // If explicit config path provided, try to load it
        if let Some(path) = config_path {
            return Self::load_from_file(path).context(format!("Failed to load config from {}", path.display()));
        }

        // Try primary location: ~/.config/whitespace/whitespace.yml
        if let Some(config_dir) = xdg_config_dir() {
            let project_name = env!("CARGO_PKG_NAME");
            let primary_config = config_dir.join(project_name).join(format!("{}.yml", project_name));
            if primary_config.exists() {
                match Self::load_from_file(&primary_config) {
                    Ok(config) => return Ok(config),
                    Err(e) => {
                        log::warn!("Failed to load config from {}: {}", primary_config.display(), e);
                    }
                }
            }
        }

        // No config file found, use defaults
        log::info!("No config file found, using defaults");
        Ok(Self::default())
    }

    fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(&path).context("Failed to read config file")?;

        let config: Self = serde_yaml::from_str(&content).context("Failed to parse config file")?;

        log::info!("Loaded config from: {}", path.as_ref().display());
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threads_config_nproc() {
        let yaml = r#"
processing:
  threads: nproc
  max-file-size: 1000000
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.processing.threads, num_cpus::get());
        assert_eq!(config.processing.max_file_size, 1000000);
    }

    #[test]
    fn test_threads_config_numeric() {
        let yaml = r#"
processing:
  threads: 8
  max-file-size: 2000000
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.processing.threads, 8);
        assert_eq!(config.processing.max_file_size, 2000000);
    }

    #[test]
    fn test_threads_config_invalid_string() {
        let yaml = r#"
processing:
  threads: "invalid"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("invalid thread value"));
    }

    #[test]
    fn test_threads_config_zero() {
        let yaml = r#"
processing:
  threads: 0
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("threads must be greater than 0"));
    }

    #[test]
    fn test_threads_config_defaults() {
        let yaml = r#"
processing:
  max-file-size: 5000000
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.processing.threads, num_cpus::get());
        assert_eq!(config.processing.max_file_size, 5000000);
    }
}

/// Runtime configuration that merges CLI arguments with file-based config.
/// This is the validated, ready-to-use configuration for the application.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Target directories to process
    pub directories: Vec<PathBuf>,
    /// Whether to perform a dry run
    pub dry_run: bool,
    /// Whether to process recursively
    pub recursive: bool,
    /// Number of threads to use
    pub threads: usize,
    /// File-based configuration (exclude patterns, etc.)
    pub file_config: Config,
}

impl RuntimeConfig {
    /// Create RuntimeConfig by merging CLI args with file config.
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        // Load file-based config
        let file_config = Config::load(cli.config.as_ref()).context("Failed to load configuration file")?;

        // Determine target directories
        let directories = if cli.directories.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            cli.directories.clone()
        };

        // Determine thread count: CLI overrides file config if explicitly set
        let threads = if cli.threads != num_cpus::get() {
            cli.threads // User explicitly set threads via CLI
        } else {
            file_config.processing.threads // Use file config value
        };

        // Validate thread count
        if threads == 0 {
            eyre::bail!("Thread count must be greater than 0");
        }

        Ok(Self {
            directories,
            dry_run: cli.dry_run,
            recursive: cli.recursive,
            threads,
            file_config,
        })
    }
}

#[cfg(test)]
mod runtime_config_tests {
    use super::*;

    fn default_cli() -> Cli {
        Cli {
            directories: vec![],
            config: None,
            dry_run: false,
            verbose: false,
            recursive: true,
            threads: num_cpus::get(),
        }
    }

    #[test]
    fn test_runtime_config_default_directory() {
        let cli = default_cli();
        let config = RuntimeConfig::from_cli(&cli).unwrap();
        assert_eq!(config.directories, vec![PathBuf::from(".")]);
    }

    #[test]
    fn test_runtime_config_explicit_directories() {
        let cli = Cli {
            directories: vec![PathBuf::from("/tmp"), PathBuf::from("/var")],
            ..default_cli()
        };
        let config = RuntimeConfig::from_cli(&cli).unwrap();
        assert_eq!(config.directories.len(), 2);
    }

    #[test]
    fn test_runtime_config_dry_run() {
        let cli = Cli {
            dry_run: true,
            ..default_cli()
        };
        let config = RuntimeConfig::from_cli(&cli).unwrap();
        assert!(config.dry_run);
    }

    #[test]
    fn test_runtime_config_threads_from_cli() {
        let cli = Cli {
            threads: 4,
            ..default_cli()
        };
        let config = RuntimeConfig::from_cli(&cli).unwrap();
        assert_eq!(config.threads, 4);
    }
}
