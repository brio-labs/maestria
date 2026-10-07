mod arguments;
mod bridge;
mod bundle;
mod error;
mod input;
mod javascript;
mod output;
mod protocol;
mod session;

use std::process::ExitCode;

use arguments::WorkerArguments;
use error::WorkerError;
use output::report_error;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => match report_error(&error) {
            Ok(()) => ExitCode::from(1),
            Err(_) => ExitCode::from(74),
        },
    }
}

async fn run() -> Result<(), WorkerError> {
    let arguments = WorkerArguments::parse()?;
    session::run(arguments).await
}
