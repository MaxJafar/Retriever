# Contributing

Build instructions and the required toolchains are in [docs/building.md](docs/building.md).
Run the Rust workspace tests, affected native build and release-tool tests before
submitting a pull request. Explain the behavior change and the checks actually run.
Distinguish compilation, fixture tests, real-library checks and measured performance.

Use isolated state directories and synthetic documents for automated tests.
Do not publish pairing/update tokens, SSH keys, personal paths, device addresses,
indexed excerpts, model weights or runtime backups. Keep the public export
allowlist current when adding a source file. `python release/export.py` validates
its contents and writes a checksum manifest.

Preserve existing library state, metadata, models and cache on updates. Changes to
on-disk formats need compatibility and recovery coverage. File writes require
validated paths, conflict checks and actionable receipts. Disk Health stays
fixture-only; tests and production code must not probe physical disk health.

Keep the approved explorer layout stable when making engine or update changes.
Make platform-specific behavior explicit, including native installer handoff and
shared-index operation. Treat document text, model output and release notes as
data, never executable instructions.
