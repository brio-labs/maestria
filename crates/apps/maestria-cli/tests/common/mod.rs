use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn bin() -> String {
    std::env::var("CARGO_BIN_EXE_maestria-cli")
        .expect("CARGO_BIN_EXE_maestria-cli not set; run via `cargo test`")
}
pub fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn maestria-cli");
    let code = if output.status.success() { 0 } else { 1 };
    (
        code,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
pub struct TempDir(PathBuf);
impl TempDir {
    pub fn new(prefix: &str) -> Self {
        let base = std::env::temp_dir();
        for n in 0..1000 {
            let path = base.join(format!("{prefix}-{n}"));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create temp dir {}: {e}", path.display()),
            }
        }
        panic!("could not create temp dir under {}", base.display());
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
pub fn write_file(parent: &Path, name: &str, contents: &str) {
    fs::write(parent.join(name), contents).expect("write test file");
}
pub fn assert_ok(args: &[&str]) -> String {
    let (code, stdout, stderr) = run(args);
    assert_eq!(
        code, 0,
        "command failed: {args:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    stdout
}
pub fn assert_ok_lines(args: &[&str], expected_lines: usize) -> String {
    let stdout = assert_ok(args);
    let actual_lines = stdout.lines().filter(|line| !line.is_empty()).count();
    assert_eq!(
        actual_lines, expected_lines,
        "unexpected stdout line count for {args:?}: {stdout}"
    );
    stdout
}
pub fn assert_init_ok(instance_path: &str, read_root: &str) {
    let stdout = assert_ok_lines(&["init", "-i", instance_path, "--read-root", read_root], 2);
    assert!(
        stdout.contains("initialized"),
        "init stdout missing 'initialized': {stdout}"
    );
    assert!(
        stdout.contains("manifest"),
        "init stdout missing 'manifest': {stdout}"
    );
}
pub fn assert_index_ok(instance_path: &str, file: &str) {
    let stdout = assert_ok_lines(&["index", "-i", instance_path, file], 1);
    assert!(
        stdout.contains("indexed"),
        "index stdout missing 'indexed': {stdout}"
    );
}
