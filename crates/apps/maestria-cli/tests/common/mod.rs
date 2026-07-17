use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn bin() -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::env::var("CARGO_BIN_EXE_maestria-cli")?)
}
pub fn run(args: &[&str]) -> Result<(i32, String, String), Box<dyn std::error::Error>> {
    let output = Command::new(bin()?).args(args).output()?;
    let code = if output.status.success() { 0 } else { 1 };
    Ok((
        code,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}
pub struct TempDir(PathBuf);
impl TempDir {
    pub fn new(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let base = std::env::temp_dir();
        for n in 0..1000 {
            let path = base.join(format!("{prefix}-{n}"));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(format!("could not create temp dir under {}", base.display()).into())
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn write_file(
    parent: &Path,
    name: &str,
    contents: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(parent.join(name), contents)?;
    Ok(())
}
pub fn assert_ok(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(args)?;
    assert_eq!(
        code, 0,
        "command failed: {args:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    Ok(stdout)
}
pub fn assert_ok_lines(
    args: &[&str],
    expected_lines: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let stdout = assert_ok(args)?;
    let actual_lines = stdout.lines().filter(|line| !line.is_empty()).count();
    assert_eq!(
        actual_lines, expected_lines,
        "unexpected stdout line count for {args:?}: {stdout}"
    );
    Ok(stdout)
}
pub fn assert_init_ok(
    instance_path: &str,
    read_root: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["init", "-i", instance_path, "--read-root", read_root], 2)?;
    assert!(
        stdout.contains("initialized"),
        "init stdout missing 'initialized': {stdout}"
    );
    assert!(
        stdout.contains("manifest"),
        "init stdout missing 'manifest': {stdout}"
    );
    Ok(())
}
pub fn assert_index_ok(instance_path: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["index", "-i", instance_path, file], 1)?;
    assert!(
        stdout.contains("indexed"),
        "index stdout missing 'indexed': {stdout}"
    );
    Ok(())
}
