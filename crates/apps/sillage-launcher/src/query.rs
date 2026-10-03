use std::cmp::Ordering;
use std::sync::LazyLock;

use crate::actions::command_definitions;
use crate::catalog::AppEntry;
use crate::model::{Action, CommandDefinition, ResultKind, SearchResult};

const MAX_RESULTS: usize = 50;
const EMPTY_APP_RESULTS: usize = 46;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchRank {
    worst_tier: u8,
    sum_tiers: u16,
}

impl MatchRank {
    fn for_terms<T>(terms: &[&str], fields: &T) -> Option<Self>
    where
        T: MatchFields,
    {
        let mut worst_tier = 0;
        let mut sum_tiers: u16 = 0;
        for term in terms {
            let tier = fields.best_tier(term)?;
            worst_tier = worst_tier.max(tier);
            sum_tiers = sum_tiers.saturating_add(u16::from(tier));
        }
        Some(Self {
            worst_tier,
            sum_tiers,
        })
    }
}

trait MatchFields {
    fn normalized_name(&self) -> &str;
    fn normalized_description(&self) -> &str;
    fn normalized_keywords(&self) -> &str;

    fn best_tier(&self, term: &str) -> Option<u8> {
        if self.normalized_name() == term {
            return Some(0);
        }
        if self.normalized_name().starts_with(term) {
            return Some(1);
        }
        if self
            .normalized_name()
            .split_whitespace()
            .any(|token| token.starts_with(term))
        {
            return Some(2);
        }
        if self.normalized_name().contains(term) {
            return Some(3);
        }
        if self.normalized_description().contains(term) || self.normalized_keywords().contains(term)
        {
            return Some(4);
        }
        if is_subsequence(term, self.normalized_name()) {
            return Some(5);
        }
        None
    }
}

impl MatchFields for AppEntry {
    fn normalized_name(&self) -> &str {
        &self.normalized_name
    }

    fn normalized_description(&self) -> &str {
        &self.normalized_description
    }

    fn normalized_keywords(&self) -> &str {
        &self.normalized_keywords
    }
}

struct IndexedCommand {
    definition: &'static CommandDefinition,
    normalized_title: String,
    normalized_subtitle: String,
    normalized_keywords: String,
}

impl MatchFields for IndexedCommand {
    fn normalized_name(&self) -> &str {
        &self.normalized_title
    }

    fn normalized_description(&self) -> &str {
        &self.normalized_subtitle
    }

    fn normalized_keywords(&self) -> &str {
        &self.normalized_keywords
    }
}

static COMMAND_INDEX: LazyLock<Vec<IndexedCommand>> = LazyLock::new(|| {
    command_definitions()
        .iter()
        .map(|definition| IndexedCommand {
            definition,
            normalized_title: definition.title.to_lowercase(),
            normalized_subtitle: definition.subtitle.to_lowercase(),
            normalized_keywords: definition.keywords.join(" ").to_lowercase(),
        })
        .collect()
});

enum Candidate<'a> {
    Application(&'a AppEntry),
    Command(&'a IndexedCommand),
}

struct RankedResult<'a> {
    result: Candidate<'a>,
    rank: MatchRank,
    normalized_name: &'a str,
    stable_id: &'a str,
    is_application: bool,
}

/// Search applications and the four built-in launcher commands.
///
/// Application fields are pre-normalized by [`AppEntry::new`]. The query and
/// fixed command metadata are normalized once per search/index lifetime,
/// respectively. Every whitespace-delimited query term must match one of the
/// indexed fields.
pub fn search_catalog(query: &str, apps: &[AppEntry]) -> Vec<SearchResult> {
    let normalized_query = query.to_lowercase();
    let terms: Vec<&str> = normalized_query.split_whitespace().collect();

    if terms.is_empty() {
        return empty_results(apps);
    }

    let mut ranked = Vec::new();
    for app in apps {
        if let Some(rank) = MatchRank::for_terms(&terms, app) {
            ranked.push(RankedResult {
                result: Candidate::Application(app),
                rank,
                normalized_name: &app.normalized_name,
                stable_id: &app.desktop_id,
                is_application: true,
            });
        }
    }
    for command in COMMAND_INDEX.iter() {
        if let Some(rank) = MatchRank::for_terms(&terms, command) {
            ranked.push(RankedResult {
                result: Candidate::Command(command),
                rank,
                normalized_name: &command.normalized_title,
                stable_id: command.definition.id,
                is_application: false,
            });
        }
    }

    ranked.sort_by(compare_ranked);
    ranked
        .into_iter()
        .take(MAX_RESULTS)
        .map(|entry| match entry.result {
            Candidate::Application(app) => application_result(app),
            Candidate::Command(command) => command_result(command),
        })
        .collect()
}

fn empty_results(apps: &[AppEntry]) -> Vec<SearchResult> {
    let mut applications: Vec<&AppEntry> = apps.iter().collect();
    applications.sort_by(|left, right| {
        left.normalized_name
            .cmp(&right.normalized_name)
            .then_with(|| left.desktop_id.cmp(&right.desktop_id))
    });

    let mut results = applications
        .into_iter()
        .take(EMPTY_APP_RESULTS)
        .map(application_result)
        .collect::<Vec<_>>();
    results.extend(COMMAND_INDEX.iter().map(command_result));
    results.truncate(MAX_RESULTS);
    results
}

fn compare_ranked(left: &RankedResult<'_>, right: &RankedResult<'_>) -> Ordering {
    left.rank
        .worst_tier
        .cmp(&right.rank.worst_tier)
        .then_with(|| left.rank.sum_tiers.cmp(&right.rank.sum_tiers))
        .then_with(|| right.is_application.cmp(&left.is_application))
        .then_with(|| left.normalized_name.cmp(right.normalized_name))
        .then_with(|| left.stable_id.cmp(right.stable_id))
}

fn application_result(app: &AppEntry) -> SearchResult {
    SearchResult {
        id: format!("app:{}", app.desktop_id),
        kind: ResultKind::Application,
        title: app.name.clone(),
        subtitle: app.description.clone(),
        icon: None,
        actions: vec![
            Action {
                id: "open".to_string(),
                title: "Open".to_string(),
                primary: true,
            },
            Action {
                id: "copy-name".to_string(),
                title: "Copy App Name".to_string(),
                primary: false,
            },
        ],
    }
}

fn command_result(command: &IndexedCommand) -> SearchResult {
    SearchResult {
        id: command.definition.id.to_string(),
        kind: ResultKind::Command,
        title: command.definition.title.to_string(),
        subtitle: command.definition.subtitle.to_string(),
        icon: None,
        actions: vec![Action {
            id: command.definition.action_id.to_string(),
            title: command.definition.action_title.to_string(),
            primary: true,
        }],
    }
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut haystack_chars = haystack.chars();
    for needle_char in needle.chars() {
        if haystack_chars.all(|haystack_char| haystack_char != needle_char) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, description: &str, keywords: &[&str]) -> AppEntry {
        AppEntry::new(
            id.to_string(),
            name.to_string(),
            description.to_string(),
            keywords.iter().map(|word| (*word).to_string()).collect(),
            None,
            format!("/usr/share/applications/{id}.desktop"),
        )
    }

    fn ids(results: &[SearchResult]) -> Vec<&str> {
        results.iter().map(|result| result.id.as_str()).collect()
    }

    #[test]
    fn ranking_distinguishes_all_name_and_metadata_tiers() {
        let apps = vec![
            app("substring", "Preterminalized", "", &[]),
            app("description", "Desk", "Terminal calculator", &[]),
            app("keyword", "Desk", "", &["terminal"]),
            app("token-prefix", "My Terminal", "", &[]),
            app("prefix", "Terminal Tools", "", &[]),
            app("exact", "Terminal", "", &[]),
            app("subsequence", "T-e-r-m-i-n-a-l", "", &[]),
        ];
        let results = search_catalog("terminal", &apps);
        assert_eq!(
            ids(&results),
            vec![
                "app:exact",
                "app:prefix",
                "app:token-prefix",
                "app:substring",
                "app:description",
                "app:keyword",
                "app:subsequence",
            ]
        );
    }

    #[test]
    fn multi_term_sort_uses_worst_tier_then_sum() {
        let apps = vec![
            app("worst", "Alpha", "Beta", &[]),
            app("lower-sum", "Alpha", "", &["beta"]),
            app("higher-sum", "Alpha Tools", "Beta", &[]),
            app("name-match", "Alpha Beta", "", &[]),
        ];
        let results = search_catalog("alpha beta", &apps);
        assert_eq!(
            ids(&results),
            vec![
                "app:name-match",
                "app:lower-sum",
                "app:worst",
                "app:higher-sum"
            ]
        );
    }

    #[test]
    fn empty_search_reserves_four_command_slots_after_46_apps() {
        let apps = (0..60)
            .map(|index| {
                app(
                    &format!("app-{index:02}"),
                    &format!("Application {index:02}"),
                    "",
                    &[],
                )
            })
            .collect::<Vec<_>>();
        let results = search_catalog("   ", &apps);
        assert_eq!(results.len(), 50);
        assert!(
            results[..46]
                .iter()
                .all(|result| matches!(result.kind, ResultKind::Application))
        );
        assert_eq!(
            ids(&results[46..]),
            vec![
                "host.open-file",
                "host.preferences",
                "host.refresh-applications",
                "host.quit",
            ]
        );
    }

    #[test]
    fn application_and_command_ties_prefer_application() {
        let apps = vec![app("preferences-app", "Preferences", "", &[])];
        let results = search_catalog("preferences", &apps);
        assert_eq!(
            ids(&results),
            vec!["app:preferences-app", "host.preferences"]
        );
    }
}
