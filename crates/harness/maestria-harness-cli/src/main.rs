use anyhow::Result;
use clap::Parser;
use maestria_domain::HarnessRunId;
use maestria_harness::LocalShellHarnessAdapter;
use maestria_ports::{HarnessAdapter, HarnessCommandClass, HarnessRequest};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(author, version, about = "Maestria Local Harness CLI")]
struct Cli {
    #[arg(short, long)]
    command: String,

    #[arg(short, long, default_value = ".")]
    working_directory: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let adapter = LocalShellHarnessAdapter;

    let request = HarnessRequest {
        run_id: HarnessRunId::new(1),
        command: cli.command,
        working_directory: cli.working_directory,
        duration_budget: Duration::from_secs(60),
        class: HarnessCommandClass::Shell,
    };

    let outcome = adapter.execute(request)?;

    println!("Exit code: {}", outcome.exit_code);
    println!("Duration: {:?}", outcome.duration);

    if !outcome.stdout.is_empty() {
        println!(
            "--- STDOUT ---\n{}",
            String::from_utf8_lossy(&outcome.stdout)
        );
    }

    if !outcome.stderr.is_empty() {
        println!(
            "--- STDERR ---\n{}",
            String::from_utf8_lossy(&outcome.stderr)
        );
    }

    Ok(())
}
