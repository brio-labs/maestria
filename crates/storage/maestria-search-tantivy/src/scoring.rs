pub(crate) fn descending_score(left: f32, right: f32) -> std::cmp::Ordering {
    match right.partial_cmp(&left) {
        Some(ordering) => ordering,
        None => std::cmp::Ordering::Equal,
    }
}

pub(crate) fn score_to_u32(score: f32) -> u32 {
    if !score.is_finite() || score <= 0.0 {
        return 0;
    }

    let scaled = score * 1_000.0;
    if scaled >= u32::MAX as f32 {
        u32::MAX
    } else {
        scaled.round() as u32
    }
}
