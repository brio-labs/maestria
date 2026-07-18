pub(super) fn relation_candidate_names(path: &str, source_qualified: Option<&str>) -> Vec<String> {
    let path = path.trim();
    if path.is_empty() {
        return Vec::new();
    }

    let mut names = Vec::new();
    let push = |name: String, out: &mut Vec<String>| {
        let normalized = name.trim();
        if normalized.is_empty() || normalized.ends_with("::*") {
            return;
        }
        out.push(normalized.to_string());
    };
    let source_scope = source_qualified
        .and_then(|source| source.rsplit_once("::").map(|(scope, _)| scope))
        .map_or("", |scope| scope);
    let module_scope = source_scope
        .rsplit_once("::")
        .map_or(source_scope, |(parent, _)| parent);

    if !path.contains("::") && !source_scope.is_empty() {
        if module_scope != source_scope {
            push(format!("{module_scope}::{path}"), &mut names);
        }
        push(format!("{source_scope}::{path}"), &mut names);
    }
    push(path.to_string(), &mut names);
    if let Some(trimmed) = path.strip_prefix("crate::") {
        push(trimmed.to_string(), &mut names);
    }
    if let Some(trimmed) = path.strip_prefix("self::") {
        if module_scope.is_empty() {
            push(trimmed.to_string(), &mut names);
        } else {
            push(format!("{module_scope}::{trimmed}"), &mut names);
        }
    }
    if let Some(trimmed) = path.strip_prefix("Self::")
        && !source_scope.is_empty()
    {
        push(format!("{source_scope}::{trimmed}"), &mut names);
    }
    if let Some(trimmed) = path.strip_prefix("super::") {
        let parent_scope = module_scope
            .rsplit_once("::")
            .map_or("", |(parent, _)| parent);
        if parent_scope.is_empty() {
            push(trimmed.to_string(), &mut names);
        } else {
            push(format!("{parent_scope}::{trimmed}"), &mut names);
        }
    }

    dedupe_ordered(names)
}

fn dedupe_ordered(mut names: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for name in names.drain(..) {
        if !deduped.contains(&name) {
            deduped.push(name);
        }
    }
    deduped
}
