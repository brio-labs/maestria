## Summary

- What changed and why?

## Invariant impact

- Which invariants in `docs/SPECS.md` are affected?

## Maestria checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-targets --all-features`
- [ ] `cargo doc --workspace --no-deps --all-features` with `RUSTDOCFLAGS="-D warnings"`
- [ ] `python3 scripts/philosophy-check.py`

## Human checks

- [ ] Domain and governance remain deterministic and side-effect free.
- [ ] Side effects remain confined to adapters/runtime.
- [ ] `README.md` or relevant docs were updated if behavior changed.
