use std::error::Error;
use std::path::PathBuf;

use maestria_domain::ArtifactId;
use maestria_parsers::*;
use maestria_ports::{FileHandle, ParseContext, Parser};

#[test]
fn rust_source_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input = b"use std::fmt;\n\npub struct Thing;\n\nimpl Thing {\n    pub fn new() -> Self { Self }\n}\n\n#[test]\nfn makes_thing() {}\n";
    let parsed = RustSourceParser::new().parse(
        FileHandle {
            path: PathBuf::from("lib.rs"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(3),
        },
    )?;
    crate::assert_debug_snapshot(
        "rust_source_parsed",
        &parsed,
        concat!(module_path!(), "::rust_source_golden_snapshot"),
        file!(),
        stringify!(&parsed),
        line!(),
    )?;
    Ok(())
}

#[test]
fn cargo_toml_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input =
        b"[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n";
    let parsed = CargoTomlParser::new().parse(
        FileHandle {
            path: PathBuf::from("Cargo.toml"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(4),
        },
    )?;
    crate::assert_debug_snapshot(
        "cargo_toml_parsed",
        &parsed,
        concat!(module_path!(), "::cargo_toml_golden_snapshot"),
        file!(),
        stringify!(&parsed),
        line!(),
    )?;
    Ok(())
}
