use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about = "Sillage daemon process")]
struct Cli {
    #[arg(short, long, default_value = ".sillage-dev")]
    instance_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    sillage_daemon::run_instance(cli.instance_dir).await
}
