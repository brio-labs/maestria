use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Sillage Launcher\nUsage: sillage-launcher [--activate|--quit|--version]");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("sillage-launcher {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    sillage_launcher::run()
}
