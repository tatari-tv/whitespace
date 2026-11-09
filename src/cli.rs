use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "whitespace",
    about = "Recursively remove trailing whitespace from files",
    version = concat!("v", env!("CARGO_PKG_VERSION")),
    after_help = "Logs are written to: ~/.local/share/whitespace/logs/whitespace.log"
)]
pub struct Cli {
    /// Target directories to process
    #[arg(help = "Target directories to process [default: .]")]
    pub directories: Vec<PathBuf>,

    /// Path to config file
    #[arg(short, long, help = "Path to config file")]
    pub config: Option<PathBuf>,

    /// Perform dry run (show what would be changed)
    #[arg(short = 'n', long, help = "Dry run - show files that would be modified")]
    pub dry_run: bool,

    /// Enable verbose output
    #[arg(short, long, help = "Enable verbose output")]
    pub verbose: bool,

    /// Process files recursively
    #[arg(short, long, help = "Recurse into subdirectories", default_value = "true")]
    pub recursive: bool,

    /// Number of parallel threads (0 = auto-detect)
    #[arg(short = 'j', long, help = "Number of parallel threads", default_value_t = num_cpus::get())]
    pub threads: usize,
}
