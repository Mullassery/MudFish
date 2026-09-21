# Contributing to Mudfish

This is a young, single-maintainer project (Phase 1 of a much larger planned
scope — see `ROADMAP_HONEST.md`). Contributions are welcome, but read that
file before proposing a large feature: most obvious-seeming gaps (browser
rendering, structured/LLM extraction, distributed crawling, a storage layer,
an MCP/agent server) are deliberately not built yet, not overlooked.

## Before you start

- For anything beyond a small fix, open an issue first describing what you
  want to change and why. This avoids wasted work on something that
  conflicts with the phased build plan or an architectural decision already
  explained in `ARCHITECTURE.md`.
- Small, obviously-correct fixes (typos, broken links, a clear bug with a
  regression test) can go straight to a pull request.

## Development setup

Requires a recent stable Rust toolchain (developed against 1.97) and, for
the Python bindings, Python >= 3.9 and [`maturin`](https://www.maturin.rs/).

```bash
git clone https://github.com/Mullassery/MudFish.git
cd MudFish
cargo build --workspace --exclude mudfish-python
```

`mudfish-python` (the PyO3 extension crate) cannot be built or tested by
plain `cargo build`/`cargo test` — an `extension-module` cdylib deliberately
omits libpython at link time, which breaks a normal Rust link step. Build
and test it via `maturin` instead:

```bash
cd crates/python
python3 -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop --release
pytest tests/ -v
```

## Before opening a pull request

Run all of the following and make sure they're clean — this is exactly what
CI checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude mudfish-python --all-targets -- -D warnings
cargo clippy -p mudfish-python -- -D warnings
cargo test --workspace --exclude mudfish-python
```

And, if you touched `crates/python`:

```bash
cd crates/python && pytest tests/ -v
```

## What a good pull request looks like

- Includes a test for the behavior it adds or fixes. If something can't
  reasonably be tested (e.g. it requires a live third-party site), say so
  explicitly in the PR description and explain how you verified it manually.
- Matches the existing crate boundaries described in `ARCHITECTURE.md`:
  `core` has zero I/O and zero dependencies on other workspace crates;
  `frontier`, `fetch`, and `parser` don't depend on each other; `engine`
  orchestrates all three and is the only thing the CLI and Python bindings
  should depend on for crawl logic.
- Does not add a new "planned"/"TODO" comment as a substitute for either
  finishing the work or filing an issue — see the honesty policy below.

## Honesty in docs and comments

This project is deliberately blunt about what is and isn't built
(`README.md`, `ROADMAP_HONEST.md`). If your change makes something partially
work, document the actual limitation plainly — not as "may need
improvement" or "future work," but as what specifically doesn't work yet.
Don't leave stub/placeholder code that looks like it works but doesn't;
either finish it or don't merge it.

## License

By contributing, you agree that your contributions are licensed under the
project's Apache-2.0 license (see `LICENSE`).
