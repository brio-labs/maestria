use crate::shell_policy::{cat_path_args, is_shell_grammar_allowed, resolve_working_directory};
use maestria_governance::Scope;
use std::path::PathBuf;

// ── shell policy grammar tests ─────────────────────────────────────────

#[test]
fn grammar_allows_echo_pwd_cat() -> Result<(), Box<dyn std::error::Error>> {
    assert!(is_shell_grammar_allowed("echo hello world"));
    assert!(is_shell_grammar_allowed("echo"));
    assert!(is_shell_grammar_allowed("pwd"));
    assert!(is_shell_grammar_allowed("cat /tmp/file.txt"));
    assert!(is_shell_grammar_allowed("cat file1.txt file2.txt"));
    assert!(is_shell_grammar_allowed("  echo  spaced  "));
    Ok(())
}

#[test]
fn grammar_rejects_unknown_commands() -> Result<(), Box<dyn std::error::Error>> {
    assert!(!is_shell_grammar_allowed("ls"));
    assert!(!is_shell_grammar_allowed("rm -rf /"));
    assert!(!is_shell_grammar_allowed("curl example.com"));
    assert!(!is_shell_grammar_allowed("bash"));
    assert!(!is_shell_grammar_allowed("sh"));
    Ok(())
}

#[test]
fn grammar_rejects_metacharacters() -> Result<(), Box<dyn std::error::Error>> {
    assert!(!is_shell_grammar_allowed("echo hello | cat"));
    assert!(!is_shell_grammar_allowed("echo hello && pwd"));
    assert!(!is_shell_grammar_allowed("echo $HOME"));
    assert!(!is_shell_grammar_allowed("echo `whoami`"));
    assert!(!is_shell_grammar_allowed("echo $(whoami)"));
    assert!(!is_shell_grammar_allowed("cat file > /dev/null"));
    assert!(!is_shell_grammar_allowed("cat < /etc/passwd"));
    assert!(!is_shell_grammar_allowed("echo hello ; rm -rf /"));
    assert!(!is_shell_grammar_allowed("cat /tmp/*"));
    assert!(!is_shell_grammar_allowed("echo ~/file"));
    assert!(!is_shell_grammar_allowed("echo hello &"));
    assert!(!is_shell_grammar_allowed("echo hello\ncat /etc/passwd"));
    assert!(!is_shell_grammar_allowed("echo hello\\nworld"));
    Ok(())
}

#[test]
fn cat_path_args_extracts_paths() -> Result<(), Box<dyn std::error::Error>> {
    let args = cat_path_args("cat /tmp/a.txt /tmp/b.txt");
    assert_eq!(args, vec!["/tmp/a.txt", "/tmp/b.txt"]);

    let args = cat_path_args("cat single.txt");
    assert_eq!(args, vec!["single.txt"]);

    let args = cat_path_args("echo hello");
    assert!(args.is_empty());

    let args = cat_path_args("pwd");
    assert!(args.is_empty());
    Ok(())
}

#[test]
fn resolve_working_directory_returns_first_read_root() -> Result<(), Box<dyn std::error::Error>> {
    let scope = Scope::new(
        vec![PathBuf::from("/workspace")],
        vec![],
        vec![],
        vec![],
        false,
    );
    let wd = resolve_working_directory(&scope)?;
    assert_eq!(wd, PathBuf::from("/workspace"));
    Ok(())
}

#[test]
fn resolve_working_directory_falls_back_when_no_roots() -> Result<(), Box<dyn std::error::Error>> {
    let scope = Scope::default();
    let wd = resolve_working_directory(&scope)?;
    assert!(!wd.as_os_str().is_empty());
    Ok(())
}
