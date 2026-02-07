mod config;
mod fs;
mod handle;
mod mapper;
mod transformer;

use std::path::PathBuf;

use clap::Parser;
use log::info;

use crate::config::Config;
use crate::fs::SecretFs;
use crate::mapper::SecretMapper;
use crate::transformer::Transformer;

/// secretfs — A FUSE filesystem that transparently replaces secrets with placeholders.
///
/// Mount a source directory at a mount point. Any file read through the mount point
/// will have secrets replaced with stable placeholders. When files are written back,
/// placeholders are restored to the original secrets.
#[derive(Parser, Debug)]
#[command(name = "secretfs", version, about)]
struct Args {
    /// Path to the source directory to mirror.
    #[arg(short, long)]
    source: PathBuf,

    /// Path to the mount point (must exist and be an empty directory).
    #[arg(short, long)]
    mount: PathBuf,

    /// Path to the secrets configuration file (YAML).
    #[arg(short, long)]
    config: PathBuf,

    /// Allow other users to access the mount (requires user_allow_other in /etc/fuse.conf).
    #[arg(long, default_value_t = false)]
    allow_other: bool,

    /// Run in foreground (don't daemonize).
    #[arg(short, long, default_value_t = true)]
    foreground: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    // Validate source directory
    if !args.source.is_dir() {
        return Err(format!("source '{}' is not a directory", args.source.display()).into());
    }

    // Validate mount point
    if !args.mount.is_dir() {
        return Err(format!("mount point '{}' is not a directory", args.mount.display()).into());
    }

    // Load config
    let config = Config::load(&args.config)?;
    info!(
        "Loaded {} secret definitions from {}",
        config.secrets.len(),
        args.config.display()
    );

    // Create mapper and transformer
    let mapper = SecretMapper::new();
    let transformer = Transformer::new(&config.secrets, mapper);

    // Create filesystem
    let source = std::fs::canonicalize(&args.source)?;
    let secretfs = SecretFs::new(source, transformer);

    // Build mount options
    let mut options = vec![
        fuser::MountOption::FSName("secretfs".to_string()),
        fuser::MountOption::DefaultPermissions,
    ];
    if args.allow_other {
        options.push(fuser::MountOption::AllowOther);
    }

    info!(
        "Mounting {} -> {}",
        args.source.display(),
        args.mount.display()
    );
    info!("Press Ctrl+C to unmount");

    // Mount and run
    fuser::mount2(secretfs, &args.mount, &options)?;

    info!("Unmounted");
    Ok(())
}
