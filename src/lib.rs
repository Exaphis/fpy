mod app;
mod cli;
mod connection;
mod custom_terminal;
mod insert_history;
mod jupyter;
mod kernel;
mod ui;

use clap::Parser;

pub async fn run() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    app::run(cli).await
}
