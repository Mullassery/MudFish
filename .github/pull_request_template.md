## What does this change do?

<!-- One or two sentences. Link an issue if there is one. -->

## Why?

<!-- What problem does this solve, or what does it enable? -->

## How was this tested?

<!--
Be specific and honest. "Ran cargo test" is not enough on its own if you
also changed the Python bindings or touched networking/SSRF code.
-->

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --workspace --exclude mudfish-python --all-targets -- -D warnings` passes
- [ ] `cargo clippy -p mudfish-python -- -D warnings` passes
- [ ] `cargo test --workspace --exclude mudfish-python` passes
- [ ] If `crates/python` changed: `pytest` passes in `crates/python`
- [ ] Manually exercised against a real crawl target (state which one)

## Anything reviewers should look at closely?

<!--
e.g. "this touches the frontier's termination protocol", "this changes
SSRF filtering behavior", "no automated test covers the new branch yet"
-->
