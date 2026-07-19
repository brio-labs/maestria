use super::*;
use maestria_code_intel::REPOSITORY_CODE_INDEX_FILENAME;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_INDEX_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_layout() -> InstanceLayout {
    let id = NEXT_INDEX_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("maestria-daemon-runtime-code-index-{id}"));
    let _ = fs::remove_dir_all(&path);
    let _ = fs::create_dir_all(&path);
    InstanceLayout::for_root(path)
}

#[test]
fn load_repository_code_index_returns_none_when_missing() -> Result<(), Box<dyn std::error::Error>>
{
    let layout = temporary_layout();
    fs::create_dir_all(&layout.system_dir)?;
    let index = load_repository_code_index_with_exclusions(&layout, None)?;
    assert!(index.is_none());
    Ok(())
}

#[test]
fn load_repository_code_index_rejects_malformed_file_as_typed_error()
-> Result<(), Box<dyn std::error::Error>> {
    let layout = temporary_layout();
    fs::create_dir_all(&layout.system_dir)?;
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    fs::write(&index_path, "not valid json")?;
    let result = load_repository_code_index_with_exclusions(&layout, None);
    assert!(result.is_err());
    assert!(matches!(
        result.err(),
        Some(maestria_code_intel::CodeIntelError::Persist { .. })
    ));
    Ok(())
}
