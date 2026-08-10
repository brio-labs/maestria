//! Source-selection policy for the index command.
//!
//! Three user-chosen tiers, all safe by construction:
//! - `Simple` indexes every collected file, no policy skipping.
//! - `Lazy` skips only large sources (slow to parse and index) and
//!   reports them so the user can re-run with `Simple`.
//! - `Smart` additionally skips high-confidence meaningless content
//!   (generated asset dumps, minified bundles); anything the policy is
//!   unsure about is indexed — a meaningful file is never treated as
//!   meaningless.

use std::fs;
use std::path::{Path, PathBuf};

/// How the batch treats sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum IndexMode {
    /// Index every collected file without policy skipping.
    Simple,
    /// Index everything fast; skip large sources and report them.
    Lazy,
    /// Skip high-confidence generated content; index everything uncertain.
    Smart,
}

/// The policy's decision for one source.
pub(crate) enum Selection {
    Index,
    Skip(&'static str),
}

/// Sources above this size are "big stuff" for `Lazy`.
const LAZY_MAX_BYTES: u64 = 1 * 1024 * 1024;

/// `Smart` never indexes sources above this size.
const SMART_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// `Smart` only calls a source minified when it is at least this large and
/// carries no line breaks in the probe window.
const MINIFIED_MIN_BYTES: u64 = 256 * 1024;

/// Probe window for the minified heuristic.
const MINIFIED_PROBE_BYTES: usize = 64 * 1024;

/// Directory components that mark machine-generated asset dumps with high
/// confidence: Minecraft-style blockstate/model/language/item/texture and
/// recipe data, coverage output, and Next.js build output. Narrow by
/// design — anything not listed here is indexed.
const GENERATED_COMPONENTS: &[&str] = &[
    "blockstates",
    "models",
    "lang",
    "items",
    "textures",
    "recipes",
    "generated",
    "coverage",
    ".next",
    "out",
];

/// Decide whether `path` (of `size` bytes) is indexed under `mode`.
pub(crate) fn select_source(path: &Path, size: u64, mode: IndexMode) -> Selection {
    match mode {
        IndexMode::Simple => Selection::Index,
        IndexMode::Lazy => {
            if size > LAZY_MAX_BYTES {
                Selection::Skip("large")
            } else {
                Selection::Index
            }
        }
        IndexMode::Smart => {
            if size > SMART_MAX_BYTES {
                return Selection::Skip("large");
            }
            if path.components().any(|component| {
                let name = component.as_os_str().to_string_lossy();
                GENERATED_COMPONENTS.iter().any(|marker| name == *marker)
            }) {
                return Selection::Skip("generated");
            }
            if size >= MINIFIED_MIN_BYTES && looks_minified(path) {
                return Selection::Skip("minified");
            }
            Selection::Index
        }
    }
}

/// A source with no line break inside the probe window is a single-line
/// machine bundle (lockfiles, minified assets) — high-confidence noise.
fn looks_minified(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut probe = [0u8; MINIFIED_PROBE_BYTES];
    let Ok(read) = std::io::Read::read(&mut file, &mut probe) else {
        return false;
    };
    !probe[..read].contains(&b'\n')
}

/// The top-level group (first component below `root`) a source belongs to.
/// Prompts and approvals are decided per group, never per file.
pub(crate) fn top_level_group(root: &Path, file: &Path) -> PathBuf {
    let Some(relative) = file.strip_prefix(root).ok() else {
        return root.to_path_buf();
    };
    let Some(first) = relative.components().next() else {
        return root.to_path_buf();
    };
    root.join(first)
}

/// Groups with at least this many sources are worth asking about.
const PROMPT_MIN_FILES: usize = 50;

/// Groups whose total size reaches this are worth asking about.
const PROMPT_MIN_BYTES: u64 = 5 * 1024 * 1024;

/// Whether a group's contribution is notable enough for an approval prompt.
pub(crate) fn is_notable_group(file_count: usize, total_bytes: u64) -> bool {
    file_count >= PROMPT_MIN_FILES || total_bytes >= PROMPT_MIN_BYTES
}

/// The interactive approval outcome for the batch.
///
/// The model is exclusion-first: everything is indexed except the subtrees
/// the user explicitly disabled. `all()` is the no-prompt path (simple
/// mode, `--yes`, scripted runs) where nothing is excluded.
#[derive(Clone, Debug, Default)]
pub(crate) struct Approval {
    skips: Vec<PathBuf>,
}

impl Approval {
    pub(crate) fn all() -> Self {
        Self::default()
    }

    pub(crate) fn add_skip(&mut self, path: PathBuf) {
        self.skips.push(path);
    }

    pub(crate) fn skips(&self) -> &[PathBuf] {
        &self.skips
    }

    /// A source is allowed when no excluded subtree contains it.
    pub(crate) fn allows(&self, path: &Path) -> bool {
        !self.skips.iter().any(|skip| path.starts_with(skip))
    }
}

/// Group the sources under `dir` by their direct child directory.
///
/// Files directly inside `dir` belong to the `None` entry and are always
/// indexed when `dir` itself is allowed.
pub(crate) fn group_by_child(dir: &Path, files: &[PathBuf]) -> Vec<(PathBuf, usize, u64)> {
    let mut groups: std::collections::BTreeMap<PathBuf, (usize, u64)> =
        std::collections::BTreeMap::new();
    for file in files {
        let Some(relative) = file.strip_prefix(dir).ok() else {
            continue;
        };
        let mut components = relative.components();
        let Some(first) = components.next() else {
            continue;
        };
        let child = dir.join(first);
        let size = fs::metadata(file).map_or(0, |metadata| metadata.len());
        let entry = groups.entry(child).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += size;
    }
    let mut ordered: Vec<(PathBuf, usize, u64)> = groups
        .into_iter()
        .map(|(path, (count, bytes))| (path, count, bytes))
        .collect();
    ordered.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.2.cmp(&left.2)));
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(components: &[&str]) -> PathBuf {
        components.iter().collect()
    }

    #[test]
    fn simple_mode_never_skips() {
        let big = Path::new("/home/user/repo/huge.md");
        assert!(matches!(
            select_source(big, 1 << 30, IndexMode::Simple),
            Selection::Index
        ));
    }

    #[test]
    fn lazy_mode_skips_only_large_sources() {
        let small = Path::new("/home/user/notes/idea.md");
        assert!(matches!(
            select_source(small, 1024, IndexMode::Lazy),
            Selection::Index
        ));
        let big = Path::new("/home/user/notes/video-transcript.md");
        assert!(matches!(
            select_source(big, 2 * 1024 * 1024, IndexMode::Lazy),
            Selection::Skip("large")
        ));
    }

    #[test]
    fn smart_mode_skips_generated_dumps_but_keeps_uncertain_sources() {
        let asset = path(&[
            "/home/user/Dev/Mod/assets",
            "minecraft",
            "blockstates",
            "allium.json",
        ]);
        assert!(matches!(
            select_source(asset.as_path(), 1024, IndexMode::Smart),
            Selection::Skip("generated")
        ));
        // Uncertain content — a user vault copy — is indexed.
        let vault = path(&["/home/user/Downloads", "Vault", "notes", "idea.md"]);
        assert!(matches!(
            select_source(vault.as_path(), 1024, IndexMode::Smart),
            Selection::Index
        ));
        let big = Path::new("/home/user/repo/big.json");
        assert!(matches!(
            select_source(big, 3 * 1024 * 1024, IndexMode::Smart),
            Selection::Skip("large")
        ));
    }

    #[test]
    fn approval_excludes_only_the_disabled_subtree() {
        let mut approval = Approval::all();
        approval.add_skip(path(&["/home/user/Dev", "Repos", "Noise"]));
        assert!(
            !approval
                .allows(path(&["/home/user/Dev", "Repos", "Noise", "data", "x.json"]).as_path())
        );
        // Siblings and unrelated subtrees stay indexed.
        assert!(approval.allows(path(&["/home/user/Dev", "Repos", "Other", "a.md"]).as_path()));
        assert!(approval.allows(path(&["/home/user/Downloads", "notes.md"]).as_path()));
        // A root-level skip covers everything below it.
        let mut root_skip = Approval::all();
        root_skip.add_skip(path(&["/home/user/Downloads"]));
        assert!(!root_skip.allows(path(&["/home/user/Downloads", "a", "b.md"]).as_path()));
        assert!(root_skip.allows(path(&["/home/user/logseq", "x.md"]).as_path()));
    }

    #[test]
    fn group_by_child_aggregates_counts_and_sizes() {
        let dir = path(&["/home/user"]);
        let files = vec![
            path(&["/home/user", "Dev", "a.md"]),
            path(&["/home/user", "Dev", "b.md"]),
            path(&["/home/user", "Notes", "c.md"]),
        ];
        let groups = group_by_child(dir.as_path(), &files);
        assert_eq!(groups.len(), 2);
        let dev = groups.iter().find(|(name, _, _)| name.ends_with("Dev"));
        assert!(dev.is_some_and(|(_, count, _)| *count == 2));
        let notes = groups.iter().find(|(name, _, _)| name.ends_with("Notes"));
        assert!(notes.is_some_and(|(_, count, _)| *count == 1));
    }
}
