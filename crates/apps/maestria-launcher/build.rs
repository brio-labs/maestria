fn main() -> Result<(), Box<dyn std::error::Error>> {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "launcher_ready",
            "search",
            "execute_action",
            "dismiss",
            "get_preferences",
            "save_preferences",
            "configure_shortcut",
            "frame_ready",
        ]),
    ))?;
    Ok(())
}
