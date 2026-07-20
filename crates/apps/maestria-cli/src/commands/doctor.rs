use anyhow::Result;
use maestria_blob_fs::FsBlobStore;
use maestria_parsers::ParserRegistry;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use std::path::PathBuf;

use crate::helpers;

pub fn run(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let manifest = helpers::load_manifest(&layout)?;
    let _sqlite_store = SqliteStore::open(&layout.database_path)?;
    let _blob_store = FsBlobStore::open(&layout.blobs_dir)?;
    let _search_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)?;
    let parser = ParserRegistry::with_defaults();
    println!("ok instance {}", layout.root.display());
    println!("ok database {}", layout.database_path.display());
    println!("ok blobs {}", layout.blobs_dir.display());
    println!(
        "ok full_text_index {}",
        layout.full_text_index_dir.display()
    );
    println!("ok parsers {}", parser.parser_count());
    println!("ocr {}", maestria_daemon::ocr_status(&manifest)?);
    Ok(())
}
