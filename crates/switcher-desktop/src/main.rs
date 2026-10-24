//! A standalone Kit desktop shell and a redacted local CLI. No remote requests.
mod app;
mod cli;
mod demo;

fn main() -> anyhow::Result<()> {
    cli::run()
}
