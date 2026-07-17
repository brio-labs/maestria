use anyhow::{Result, anyhow};
use maestria_domain::{DomainEvent, TaskId};
use std::path::PathBuf;

use crate::helpers;

pub fn run_evidence_coverage(instance_dir: PathBuf, task_id: u64) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let state = maestria_daemon::load_kernel_state(&layout)?;
    let task_id = TaskId::new(task_id);
    let task = state
        .tasks
        .get(&task_id)
        .ok_or_else(|| anyhow!("task {task_id} was not found"))?;
    println!("task_id={} status={:?}", task.id, task.status);
    println!("evidence_ids={:?}", task.evidence_ids);
    println!("evidence_count={}", task.evidence_ids.len());
    match task.validation_report_id {
        Some(report_id) => {
            let report = state.validation_reports.get(&report_id).ok_or_else(|| {
                anyhow!("validation report {report_id} for task {task_id} was not found")
            })?;
            println!(
                "validation_report={} passed={} warnings={:?}",
                report_id, report.passed, report.warnings
            );
        }
        None => println!("validation_report=none"),
    }
    let events = super::load_events(&layout)?;
    let search = events.iter().rev().find_map(|event| match &event.event {
        DomainEvent::SearchKnowledgeCompleted {
            task_id: Some(found_task),
            plan,
            outcome,
        } if *found_task == task_id => Some((plan.clone(), outcome.clone())),
        _ => None,
    });
    if let Some((plan, outcome)) = search {
        let plan = plan.as_deref().ok_or_else(|| {
            anyhow!("task {task_id} has a non-reproducible search: durable plan is missing")
        })?;
        let trace = outcome.trace_data.as_deref().ok_or_else(|| {
            anyhow!("task {task_id} has a non-reproducible search: trace payload is missing")
        })?;
        outcome
            .verify_compatibility(plan)
            .map_err(|error| anyhow!("task {task_id} has a non-reproducible search: {error}"))?;
        println!("search_query={}", plan.original_query);
        println!("search_trace={}", outcome.trace);
        println!("search_status={:?}", outcome.status);
        println!("coverage_percent={}%", outcome.coverage.percent_covered);
        println!("coverage_gaps={:?}", outcome.coverage.gaps_identified);
        println!("required_claims={:?}", outcome.coverage.required_claims);
        println!(
            "required_subquestions={:?}",
            outcome.coverage.required_subquestions
        );
        println!("conflicts={:?}", outcome.conflicts);
        println!("stop_reason={:?}", trace.stop_reason);
        println!("missing_evidence={:?}", trace.missing_evidence);
    } else {
        println!("search_coverage=unavailable: no durable search outcome is linked to task");
    }
    Ok(())
}
