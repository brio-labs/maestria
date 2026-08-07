use crate::CodeIntelError;
use crate::symbols::markers::{attr_bench, attr_test};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct ModuleContext {
    pub(crate) stack: Vec<String>,
    pub(crate) is_test: bool,
    pub(crate) is_bench: bool,
}

pub(crate) fn collect_rust_files(
    path: &Path,
    root: &Path,
    out: &mut Vec<PathBuf>,
    excluded_patterns: &[String],
    module_context: &ModuleContext,
    module_contexts: &mut BTreeMap<PathBuf, ModuleContext>,
    parents: &mut BTreeMap<PathBuf, PathBuf>,
) -> Result<(), CodeIntelError> {
    if is_excluded(path, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| CodeIntelError::Io {
        operation: "metadata".to_string(),
        path: path.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        if is_rust_file(path) {
            collect_source_and_modules(
                path,
                root,
                out,
                excluded_patterns,
                module_context,
                module_contexts,
                parents,
            )?;
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|error| CodeIntelError::Io {
        operation: "read directory".to_string(),
        path: path.to_string_lossy().into_owned(),
        details: error.to_string(),
    })? {
        let entry = entry.map_err(|error| CodeIntelError::Io {
            operation: "read directory entry".to_string(),
            path: path.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        let child = entry.path();
        if let Some(name) = child.file_name().and_then(|name| name.to_str())
            && (name.starts_with('.') || name == "target")
        {
            continue;
        }
        collect_rust_files(
            &child,
            root,
            out,
            excluded_patterns,
            module_context,
            module_contexts,
            parents,
        )?;
    }
    Ok(())
}

pub(crate) fn collect_source_and_modules(
    path: &Path,
    root: &Path,
    out: &mut Vec<PathBuf>,
    excluded_patterns: &[String],
    module_context: &ModuleContext,
    module_contexts: &mut BTreeMap<PathBuf, ModuleContext>,
    parents: &mut BTreeMap<PathBuf, PathBuf>,
) -> Result<(), CodeIntelError> {
    let canonical = path.canonicalize().map_err(|error| CodeIntelError::Io {
        operation: "canonicalize Rust module".to_string(),
        path: path.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    if !canonical.starts_with(root) {
        return Err(CodeIntelError::Identity {
            context: "validate Rust module scope".to_string(),
            details: format!("module {} points outside repository", path.display()),
        });
    }
    if out.iter().any(|existing| existing == &canonical) {
        return Ok(());
    }
    out.push(canonical.clone());
    module_contexts.insert(canonical.clone(), module_context.clone());
    let source = fs::read_to_string(&canonical).map_err(|error| CodeIntelError::Io {
        operation: "read Rust source for module discovery".to_string(),
        path: canonical.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let file = syn::parse_file(&source).map_err(|error| CodeIntelError::Parse {
        context: format!(
            "parse Rust source for module discovery: {}",
            canonical.display()
        ),
        details: error.to_string(),
    })?;
    let base_dir = match canonical.parent() {
        Some(base) => base.to_path_buf(),
        None => PathBuf::from("."),
    };
    let mut discovery = ModuleDiscovery {
        root,
        declaring_file: &canonical,
        out,
        excluded_patterns,
        module_contexts,
        parents,
    };
    follow_module_items(&file.items, &base_dir, module_context, &mut discovery)
}

/// Shared state of one module-discovery walk over a single source file: the
/// repository scope, the file that declared the walked module scope (recorded
/// as the parent of every file-backed child), and the accumulators.
struct ModuleDiscovery<'a> {
    root: &'a Path,
    declaring_file: &'a Path,
    out: &'a mut Vec<PathBuf>,
    excluded_patterns: &'a [String],
    module_contexts: &'a mut BTreeMap<PathBuf, ModuleContext>,
    parents: &'a mut BTreeMap<PathBuf, PathBuf>,
}

/// Follow `mod` declarations inside a module scope the way rustc does: inline
/// `mod { }` blocks are entered recursively (their file-backed child mods
/// resolve against the block's base directory, redirected by a `#[path]`
/// attribute on the block itself), while file-backed `mod x;` declarations
/// resolve against the current base directory and recurse into their own
/// file. Macro bodies are token trees and therefore invisible here.
fn follow_module_items(
    items: &[syn::Item],
    base_dir: &Path,
    module_context: &ModuleContext,
    discovery: &mut ModuleDiscovery<'_>,
) -> Result<(), CodeIntelError> {
    for item in items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        let mut child_context = module_context.clone();
        child_context.stack.push(module.ident.to_string());
        child_context.is_test |= attr_test(&module.attrs);
        child_context.is_bench |= attr_bench(&module.attrs);
        if let Some((_, block_items)) = &module.content {
            let nested_base = inline_mod_base(base_dir, module);
            follow_module_items(block_items, &nested_base, &child_context, discovery)?;
            continue;
        }
        let module_path = external_module_path(base_dir, module);
        if !module_path.exists() || is_excluded(&module_path, discovery.excluded_patterns) {
            continue;
        }
        let child = module_path
            .canonicalize()
            .map_err(|error| CodeIntelError::Io {
                operation: "canonicalize external Rust module".to_string(),
                path: module_path.to_string_lossy().into_owned(),
                details: error.to_string(),
            })?;
        if !child.starts_with(discovery.root) {
            return Err(CodeIntelError::Identity {
                context: "validate external Rust module scope".to_string(),
                details: format!("module {} points outside repository", module.ident),
            });
        }
        discovery
            .parents
            .insert(child.clone(), discovery.declaring_file.to_path_buf());
        collect_source_and_modules(
            &child,
            discovery.root,
            discovery.out,
            discovery.excluded_patterns,
            &child_context,
            discovery.module_contexts,
            discovery.parents,
        )?;
    }
    Ok(())
}

/// Base directory for file-backed mods nested in an inline `mod { }` block:
/// rustc resolves them relative to the directory of the block's `#[path]`
/// value when present, otherwise relative to the enclosing base directory.
fn inline_mod_base(base_dir: &Path, module: &syn::ItemMod) -> PathBuf {
    let Some(value) = path_attr_value(&module.attrs) else {
        return base_dir.to_path_buf();
    };
    match base_dir.join(value).parent() {
        Some(parent) => parent.to_path_buf(),
        None => base_dir.to_path_buf(),
    }
}

fn path_attr_value(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attribute| {
        if !attribute.path().is_ident("path") {
            return None;
        }
        let syn::Meta::NameValue(value) = &attribute.meta else {
            return None;
        };
        let syn::Expr::Lit(expression) = &value.value else {
            return None;
        };
        let syn::Lit::Str(path) = &expression.lit else {
            return None;
        };
        Some(path.value())
    })
}

fn external_module_path(base_dir: &Path, module: &syn::ItemMod) -> PathBuf {
    if let Some(path) = path_attr_value(&module.attrs) {
        return base_dir.join(path);
    }
    let module_name = module.ident.to_string();
    let flat = base_dir.join(format!("{module_name}.rs"));
    if flat.exists() {
        flat
    } else {
        base_dir.join(&module_name).join("mod.rs")
    }
}

fn is_excluded(path: &Path, patterns: &[String]) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == ".git"
            || name == ".ssh"
            || name == ".gnupg"
            || name == "secrets"
            || name == "target"
            || name == "node_modules"
            || name == "dist"
            || name == "build"
            || patterns.iter().any(|pattern| {
                pattern.as_str() == name
                    || (pattern == ".env.*" && name.starts_with(".env."))
                    || (pattern == "*.pem" && name.ends_with(".pem"))
                    || (pattern == "*.key" && name.ends_with(".key"))
            })
    })
}

fn is_rust_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("rs")
}
