mod app;
mod cli;
mod connection;
mod extensions;
mod frontend_magic;
mod history;
mod jupyter;
mod kernel;
mod ui;

use clap::Parser;

pub async fn run() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    app::run(cli).await
}

pub fn display_fixture_json(scenario: &str, width: u16, height: u16) -> anyhow::Result<String> {
    ui::display::fixture_json(scenario, width, height)
}

pub fn display_fixture_sequence_json(
    sequence: &str,
    width: u16,
    height: u16,
) -> anyhow::Result<String> {
    ui::display::fixture_sequence_json(sequence, width, height)
}
