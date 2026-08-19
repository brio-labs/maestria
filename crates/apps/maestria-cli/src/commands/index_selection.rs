//! Whitelist-first selection planning for the index command.
//!
//! Builds the approved whitelist from the candidate tree: `Recommended`
//! directories auto-include, `Noise` subtrees are excluded, and the rest
//! are prompted (or auto-approved on scripted runs). The plan maps every
//! approved path to the policy its files run under.

use anyhow::{Result, anyhow};
use maestria_index_selection::{CandidateDir, Class, IndexPolicy, is_notable_group};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Count one policy skip under its reason, preserving insertion order.
pub(super) fn record_skip(skipped: &mut Vec<(&'static str, usize)>, reason: &'static str) {
    if let Some((_, count)) = skipped.iter_mut().find(|(name, _)| *name == reason) {
        *count += 1;
    } else {
        skipped.push((reason, 1));
    }
}

/// The whitelist under construction: approved paths and their policies.
#[derive(Default)]
pub(super) struct SelectionPlan {
    includes: Vec<PathBuf>,
    policies: BTreeMap<PathBuf, IndexPolicy>,
}

impl SelectionPlan {
    /// Approve one candidate: record its path and the policy it runs under.
    fn include(&mut self, node: &CandidateDir, batch_policy: Option<IndexPolicy>) {
        let policy = node_policy(node, batch_policy);
        self.include_with_policy(&node.path, policy);
    }

    /// Approve `path` under an explicit policy (a prompted `Y` keeps the
    /// displayed policy, which `p` may have toggled).
    fn include_with_policy(&mut self, path: &Path, policy: IndexPolicy) {
        self.includes.push(path.to_path_buf());
        self.policies.insert(path.to_path_buf(), policy);
    }

    /// Approve a direct file target under an explicit policy.
    pub(super) fn approve_path(&mut self, path: &Path, policy: IndexPolicy) {
        self.include_with_policy(path, policy);
    }

    /// The per-file policy for `file`: the deepest approved ancestor's
    /// override, or the batch policy, or no filtering at all.
    pub(super) fn file_policy(
        &self,
        file: &Path,
        batch_policy: Option<IndexPolicy>,
    ) -> IndexPolicy {
        let deepest_override = self
            .includes
            .iter()
            .filter(|include| file.starts_with(include))
            .max_by_key(|include| include.as_os_str().len())
            .and_then(|include| self.policies.get(include).copied());
        if let Some(policy) = deepest_override.or(batch_policy) {
            policy
        } else {
            IndexPolicy::everything()
        }
    }

    /// Whether any approved path contains `file`.
    pub(super) fn allows(&self, file: &Path) -> bool {
        self.includes
            .iter()
            .any(|include| file.starts_with(include))
    }

    /// The approved paths.
    pub(super) fn includes(&self) -> &[PathBuf] {
        &self.includes
    }

    /// The per-path policy overrides.
    pub(super) fn policies(&self) -> &BTreeMap<PathBuf, IndexPolicy> {
        &self.policies
    }
}

/// Deepest drill-down level (root group = 1, so level 4 reaches folders
/// inside a repository).
const MAX_PROMPT_DEPTH: usize = 4;
const MAX_PROMPT_CHILDREN: usize = 6;

/// The default policy for a node: the forced batch policy, or the
/// classification default.
fn node_policy(node: &CandidateDir, batch_policy: Option<IndexPolicy>) -> IndexPolicy {
    if let Some(policy) = batch_policy {
        policy
    } else {
        node.policy
    }
}

/// Ask the user how to treat the candidate subtree, bounded so a batch
/// never becomes a questionnaire:
/// - `Recommended` directories are auto-included without a prompt (their
///   default policy indexes everything);
/// - `Noise` directories that are not notable are excluded silently;
/// - `Maybe` (or notable `Noise`) directories are prompted: `Y` includes,
///   `n` excludes the subtree, `l` drills one level deeper (up to
///   [`MAX_PROMPT_DEPTH`], prompting at most [`MAX_PROMPT_CHILDREN`]
///   notable children per level — the rest inherit the parent's undecided
///   drill and stay unapproved), `p` toggles the directory policy between
///   its default and `everything()`, `a` accepts everything from here on,
///   `q` aborts.
fn prompt_candidate(
    node: &CandidateDir,
    depth: usize,
    batch_policy: Option<IndexPolicy>,
    plan: &mut SelectionPlan,
    accept_all: &mut bool,
) -> Result<()> {
    if *accept_all {
        plan.include(node, batch_policy);
        return Ok(());
    }
    if node.class == Class::Recommended {
        plan.include(node, batch_policy);
        return Ok(());
    }
    if node.class == Class::Noise && !is_notable_group(node.file_count, node.total_bytes) {
        return Ok(());
    }
    let mut policy = node_policy(node, batch_policy);
    loop {
        let size_mb = node.total_bytes as f64 / (1024.0 * 1024.0);
        let options = if depth < MAX_PROMPT_DEPTH {
            "[Y/n/l/p/a/q]"
        } else {
            "[Y/n/p/a/q]"
        };
        println!(
            "Index everything under {}? ({} files, {size_mb:.1} MB) policy: {} {options}",
            node.path.display(),
            node.file_count,
            policy.display()
        );
        print!("> ");
        // Flushing stdout for an interactive prompt is best-effort; a flush
        // failure (e.g., broken pipe) is non-fatal and should not abort the
        // selection flow.
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            return Err(anyhow!("failed to read approval answer"));
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => {
                plan.include_with_policy(&node.path, policy);
                return Ok(());
            }
            "n" | "no" => return Ok(()),
            "l" | "list" if depth < MAX_PROMPT_DEPTH => {
                let mut prompted = 0usize;
                for child in &node.children {
                    if child.class == Class::Recommended {
                        plan.include(child, batch_policy);
                        continue;
                    }
                    // Children below the notability bar (and beyond the
                    // prompt cap) inherit the parent's undecided drill:
                    // nothing is whitelisted without approval.
                    if !is_notable_group(child.file_count, child.total_bytes)
                        || prompted >= MAX_PROMPT_CHILDREN
                    {
                        continue;
                    }
                    prompted += 1;
                    prompt_candidate(child, depth + 1, batch_policy, plan, accept_all)?;
                }
                return Ok(());
            }
            "l" | "list" => {
                println!("This directory is already at the deepest drill-down level.");
            }
            "p" | "policy" => {
                let default = node_policy(node, batch_policy);
                policy = if policy == IndexPolicy::everything() {
                    default
                } else {
                    IndexPolicy::everything()
                };
            }
            "a" | "all" => {
                *accept_all = true;
                plan.include(node, batch_policy);
                return Ok(());
            }
            "q" | "quit" => return Err(anyhow!("aborted by user")),
            _ => {}
        }
    }
}

/// Non-interactive approval: every candidate is included except `Noise`
/// subtrees; `--yes` includes those too. `Recommended` subtrees stop the
/// walk, matching the interactive behavior.
pub(super) fn approve_scripted(
    tree: &CandidateDir,
    batch_policy: Option<IndexPolicy>,
    yes: bool,
    plan: &mut SelectionPlan,
) {
    if tree.class == Class::Noise && !yes {
        return;
    }
    plan.include(tree, batch_policy);
    if tree.class == Class::Recommended {
        return;
    }
    for child in &tree.children {
        approve_scripted(child, batch_policy, yes, plan);
    }
}

/// Build the plan interactively: walk the top-level groups of `tree` and
/// prompt for every candidate that needs a decision.
pub(super) fn approve_interactively(
    tree: &CandidateDir,
    batch_policy: Option<IndexPolicy>,
) -> Result<SelectionPlan> {
    let mut plan = SelectionPlan::default();
    let mut accept_all = false;
    for child in &tree.children {
        prompt_candidate(child, 1, batch_policy, &mut plan, &mut accept_all)?;
    }
    Ok(plan)
}
