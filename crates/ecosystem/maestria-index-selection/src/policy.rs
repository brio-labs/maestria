//! Per-file selection policy: three independent switches the user can
//! toggle per directory.

use std::fs;
use std::path::{Path, PathBuf};

/// Skip files larger than this many bytes. 0 = no limit.
const LARGE_MIN_BYTES: u64 = 1024 * 1024;

/// A source is only called minified when it is at least this large and
/// carries no line breaks in the probe window.
const MINIFIED_MIN_BYTES: u64 = 256 * 1024;

/// Probe window for the minified heuristic.
const MINIFIED_PROBE_BYTES: usize = 64 * 1024;

/// Three independent per-directory policy switches.
///
/// `skip_generated` is directory-level: generated dumps are excluded by
/// the candidate whitelist, not by per-file selection (a dump directory
/// is classified `Noise` and never whitelisted unless explicitly
/// approved). `max_file_bytes` and `skip_minified` are applied per file
/// by [`select_source`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct IndexPolicy {
    /// Skip files larger than this many bytes. 0 = no limit.
    pub max_file_bytes: u64,
    /// Skip files under a generated dump (single-extension dumps, rule below).
    pub skip_generated: bool,
    /// Skip minified single-line bundles (rule below).
    pub skip_minified: bool,
}

impl IndexPolicy {
    /// All switches off: index everything.
    pub fn everything() -> Self {
        Self::default()
    }

    /// The filtered defaults: skip large files, generated dumps, and
    /// minified bundles.
    pub fn filtered() -> Self {
        Self {
            max_file_bytes: LARGE_MIN_BYTES,
            skip_generated: true,
            skip_minified: true,
        }
    }

    /// Whether any switch is active.
    pub fn is_filtered(&self) -> bool {
        self.max_file_bytes > 0 || self.skip_generated || self.skip_minified
    }

    /// Human-readable summary, used by the CLI prompt.
    pub fn display(&self) -> String {
        if !self.is_filtered() {
            return "index everything".to_string();
        }
        let mut parts = Vec::new();
        if self.max_file_bytes > 0 {
            parts.push(format!("skip >{}", human_bytes(self.max_file_bytes)));
        }
        if self.skip_generated {
            parts.push("generated dumps".to_string());
        }
        if self.skip_minified {
            parts.push("minified bundles".to_string());
        }
        parts.join(", ")
    }
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 && bytes.is_multiple_of(1024 * 1024) {
        format!("{}MiB", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{}KiB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}

/// The policy's decision for one source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Index,
    Skip(&'static str),
}

/// Decide whether `path` (of `size` bytes) is indexed under `policy`.
///
/// Rules, in order: files above `max_file_bytes` (when set) are `large`;
/// single-line bundles of at least 256 KiB are `minified` when the switch
/// is on; everything else is indexed. The generated rule is
/// directory-level and applied by the selection pass (the whitelist).
pub fn select_source(path: &Path, size: u64, policy: IndexPolicy) -> Selection {
    if policy.max_file_bytes > 0 && size > policy.max_file_bytes {
        return Selection::Skip("large");
    }
    if policy.skip_minified && size >= MINIFIED_MIN_BYTES && looks_minified(path) {
        return Selection::Skip("minified");
    }
    Selection::Index
}

/// A source with no line break inside the probe window is a single-line
/// machine bundle (lockfiles, minified assets) — high-confidence noise.
pub(crate) fn looks_minified(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut probe = [0u8; MINIFIED_PROBE_BYTES];
    let Ok(read) = std::io::Read::read(&mut file, &mut probe) else {
        return false;
    };
    !probe[..read].contains(&b'\n')
}

/// Groups with at least this many sources are worth asking about.
const PROMPT_MIN_FILES: usize = 50;

/// Groups whose total size reaches this are worth asking about.
const PROMPT_MIN_BYTES: u64 = 5 * 1024 * 1024;

/// Whether a group's contribution is notable enough for an approval prompt.
pub fn is_notable_group(file_count: usize, total_bytes: u64) -> bool {
    file_count >= PROMPT_MIN_FILES || total_bytes >= PROMPT_MIN_BYTES
}

/// Group the sources under `dir` by their direct child directory.
///
/// Files directly inside `dir` belong to the `None` entry and are always
/// indexed when `dir` itself is allowed.
pub fn group_by_child(dir: &Path, files: &[PathBuf]) -> Vec<(PathBuf, usize, u64)> {
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
