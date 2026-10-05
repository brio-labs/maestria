use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::Duration,
};

const SEARCH_BIN: &str = env!("CARGO_BIN_EXE_sillage-search");

struct PrivateTempDir(PathBuf);

impl PrivateTempDir {
    fn new() -> io::Result<Self> {
        let parent = std::env::temp_dir();
        for index in 0..1000 {
            let path = parent.join(format!(
                "sillage-search-review-{}-{index}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a private temporary directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct SearchDaemon(Child);

impl SearchDaemon {
    fn start(instance: &Path) -> io::Result<Self> {
        let child = Command::new(SEARCH_BIN)
            .args(["start", "--instance-dir"])
            .arg(instance)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self(child))
    }

    fn wait_ready(&mut self, instance: &Path) -> io::Result<()> {
        for _ in 0..120 {
            let output = run(&grant_list_args(instance))?;
            if output.status.success() {
                return Ok(());
            }
            if self.0.try_wait()?.is_some() {
                return Err(io::Error::other("provider stopped before becoming ready"));
            }
            thread::sleep(Duration::from_millis(25));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "provider did not become ready",
        ))
    }
}

impl Drop for SearchDaemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run(args: &[OsString]) -> io::Result<Output> {
    Command::new(SEARCH_BIN).args(args).output()
}

fn grant_list_args(instance: &Path) -> Vec<OsString> {
    vec![
        "owner".into(),
        "grant".into(),
        "list".into(),
        "--instance-dir".into(),
        instance.as_os_str().to_owned(),
    ]
}

fn shared_grant_args(
    args: &mut Vec<OsString>,
    realm: &str,
    root: &Path,
    max_results: &str,
    expiry: &str,
) {
    args.extend([
        "--consumer-realm".into(),
        realm.into(),
        "--access".into(),
        "search-only".into(),
        "--max-sensitivity".into(),
        "public".into(),
        "--max-results".into(),
        max_results.into(),
        "--max-evidence-bytes".into(),
        "4096".into(),
        "--read-root".into(),
        root.as_os_str().to_owned(),
        "--expires-in-seconds".into(),
        expiry.into(),
    ]);
}

fn review_args(
    instance: &Path,
    realm: &str,
    root: &Path,
    max_results: &str,
    expiry: &str,
) -> Vec<OsString> {
    let mut args = vec![
        "owner".into(),
        "grant".into(),
        "review-external".into(),
        "--instance-dir".into(),
        instance.as_os_str().to_owned(),
    ];
    shared_grant_args(&mut args, realm, root, max_results, expiry);
    args.extend([
        "--consumer-label".into(),
        "org.example.provider-fixture".into(),
    ]);
    args
}

fn create_args(
    instance: &Path,
    realm: &str,
    root: &Path,
    credential: &Path,
    max_results: &str,
    expiry: &str,
) -> Vec<OsString> {
    let mut args = vec![
        "owner".into(),
        "grant".into(),
        "create-external".into(),
        "--instance-dir".into(),
        instance.as_os_str().to_owned(),
        "--credential-file".into(),
        credential.as_os_str().to_owned(),
    ];
    shared_grant_args(&mut args, realm, root, max_results, expiry);
    args
}

fn assert_rejected_without_echo(output: Output) {
    assert!(
        !output.status.success(),
        "invalid grant request was accepted"
    );
    assert!(output.stdout.is_empty(), "rejected request wrote a preview");
}

#[test]
fn owner_grant_review_is_read_only_safe_and_uses_creation_validation()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = PrivateTempDir::new()?;
    let instance = temporary.path().join("provider-instance");
    let root = temporary.path().join("approved root");
    let outside_root = temporary.path().join("unapproved root");
    fs::create_dir(&root)?;
    fs::create_dir(&outside_root)?;
    let credential_directory = temporary.path().join("private-credentials");
    let credential = credential_directory.join("extension.key");
    let realm = "c".repeat(64);

    let initialized = run(&[
        "init".into(),
        "--instance-dir".into(),
        instance.as_os_str().to_owned(),
        "--read-root".into(),
        root.as_os_str().to_owned(),
    ])?;
    assert!(
        initialized.status.success(),
        "temporary provider initialization failed"
    );

    let mut daemon = SearchDaemon::start(&instance)?;
    daemon.wait_ready(&instance)?;
    let before = run(&grant_list_args(&instance))?;
    assert!(
        before.status.success(),
        "provider grant inventory was unavailable"
    );
    assert!(
        before.stdout.is_empty(),
        "fresh provider had unexpected grants"
    );

    let preview = run(&review_args(&instance, &realm, &root, "2", "1800"))?;
    assert!(
        preview.status.success(),
        "valid external grant review failed"
    );
    let preview_text = std::str::from_utf8(&preview.stdout)?;
    assert!(
        !preview_text.contains(&realm),
        "review exposed the consumer realm"
    );
    assert!(!preview_text.contains("consumer_realm="));
    assert!(!preview_text.contains("provider_realm="));
    assert!(!preview_text.contains("token_digest"));
    assert!(!preview_text.contains("credential_file"));
    assert!(!preview_text.contains(&credential.to_string_lossy().to_string()));
    assert!(
        !credential_directory.exists(),
        "review created a credential directory"
    );
    assert!(!credential.exists(), "review created a credential file");

    let after_preview = run(&grant_list_args(&instance))?;
    assert!(
        after_preview.status.success(),
        "provider grant inventory failed after review"
    );
    assert!(
        before.stdout == after_preview.stdout,
        "read-only review changed the provider grant inventory"
    );

    for (max_results, expiry) in [("101", "1800"), ("2", "0")] {
        for review in [true, false] {
            let args = if review {
                review_args(&instance, &realm, &root, max_results, expiry)
            } else {
                create_args(&instance, &realm, &root, &credential, max_results, expiry)
            };
            assert_rejected_without_echo(run(&args)?);
        }
    }

    let invalid_review = run(&review_args(&instance, &realm, &outside_root, "2", "1800"))?;
    let invalid_create = run(&create_args(
        &instance,
        &realm,
        &outside_root,
        &credential,
        "2",
        "1800",
    ))?;
    assert_rejected_without_echo(invalid_review);
    assert_rejected_without_echo(invalid_create);
    assert!(
        !credential_directory.exists(),
        "rejected grant creation wrote credentials"
    );

    let after_rejections = run(&grant_list_args(&instance))?;
    assert!(
        after_rejections.status.success(),
        "provider grant inventory failed after rejection"
    );
    assert!(
        before.stdout == after_rejections.stdout,
        "invalid review or creation changed the provider grant inventory"
    );
    Ok(())
}
