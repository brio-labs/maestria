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
- [ ] `python3 -m unittest discover -s scripts -p 'test_*.py'`

## Human checks

- [ ] Domain and governance remain deterministic and side-effect free.
- [ ] Side effects remain confined to adapters/runtime.
- [ ] Each module owns one responsibility at one architectural layer; new concepts have explicit sibling boundaries.
- [ ] Public façades expose stable boundaries, and cross-concern behavior uses typed APIs, traits, or effects.
- [ ] `README.md` or relevant docs were updated if behavior changed.
