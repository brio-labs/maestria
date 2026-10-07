use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|arg| arg == "--read-clipboard") {
        if args.len() != 2 || args[1] != "--read-clipboard" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--read-clipboard must be used by itself",
            )
            .into());
        }
        sillage_launcher::export_clipboard_text()?;
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Sillage Launcher\nUsage: sillage-launcher [--activate|--background|--quit|--version]"
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("sillage-launcher {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    sillage_launcher::run()
}
