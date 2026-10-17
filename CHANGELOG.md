# Changelog

## 0.13.0-alpha.21-native — public preview preparation

- Added GitHub release checks and update notifications in the existing status bar.
- Added preview selection, reminders, version skipping, cancellable downloads and
  size/SHA-256 verification before native installer handoff.
- Added local-only engine shutdown authorization, busy-operation checks and a
  database checkpoint before installation. Shared clients cannot stop the server.
- Persisted custom library/shared-index bootstrap settings outside app bundles.
- Consolidated the current Rust engine and native clients into a source allowlist,
  with public build instructions, dependency attribution and draft-release CI.
- Kept model weights separate from application updates.

## Earlier native previews

Established the WinUI 3 and SwiftUI clients, native EmbeddingGemma/Laya retrieval,
shared-library mode, real Disk Capacity scans, in-app folder selection, native
Quick Search, file-operation previews and lossless transfer/storage compression.
Disk Health remains fixture-only. Private development and acceptance records are
not part of the public release history.
