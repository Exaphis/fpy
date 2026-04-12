use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "fpy",
    version,
    about = "Minimal Rust terminal frontend for ipykernel"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Run(RunArgs),
    Attach(AttachArgs),
}

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    #[arg(long, default_value = "python3")]
    pub python: String,

    #[arg(long = "kernel-arg")]
    pub kernel_args: Vec<String>,
}

#[derive(Debug, Clone, Args)]
pub struct AttachArgs {
    #[arg(long)]
    pub connection_file: PathBuf,
}
