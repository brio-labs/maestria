/// Autonomy profile governing effect execution latitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomyProfile {
    ReadOnly,
    Assisted,
    ScopedAutonomy,
    StrictResearch,
    TrustedWorkspace,
}
