use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Maestria Launcher\nUsage: maestria-launcher [--activate|--quit|--version]");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("maestria-launcher {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    maestria_launcher::run()
}
