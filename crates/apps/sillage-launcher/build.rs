fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint_build::compile("ui/launcher.slint")?;
    Ok(())
}
