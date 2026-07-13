// Dispatch implementations moved to focused sibling modules.
// See dispatch_crud.rs (single-event dispatch helpers) and
// dispatch_complex.rs (multi-event/complex dispatch helpers).
// This file is kept as a thin façade — all `impl KernelState` blocks
// reside in the sibling modules and are compiled together by input.rs.
