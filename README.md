# Retriever

A native file browser and local search engine for Windows and macOS. Retriever
combines folder browsing, indexed filename and text search, optional semantic
ranking, file transfers and real disk-capacity analysis in one desktop app.

The current public preview is **0.13.0-alpha.21-native**. Windows x64 and Apple
Silicon macOS 14+ are the desktop targets. This repository contains the Rust
engine, WinUI 3 and SwiftUI clients, native AI integration and release tooling.
The app does not require Python or a model server at runtime.

- Browse local and paired remote folders, with native Quick Search.
- Search indexed filenames and content; add the native EmbeddingGemma/Laya
  bundle for meaning-based ranking.
- Share a coordinator's index across devices, preserving one metadata/cache store.
- Preview and run copy/move operations, with lossless adaptive transfer compression.
- Inspect actual folder sizes in Disk Capacity. **Disk Health is fixture-only.**
- Receive in-app GitHub release notifications, verify downloads and open the
  native installer without resetting library data.

## Install and update

Use the installers attached to a [published release](https://github.com/MaxJafar/Retriever/releases).
The first public preview must be built and published before downloads are available.
Preview binaries may be unsigned; only signed/notarized builds have verified
publisher identity at the operating-system level. Model weights are separate
from the app and are governed by their own licenses.

Open Settings → Updates to check, download, defer or skip an update. Windows
opens the installer; Mac opens a disk image for replacing the app in Applications.
Read [update behavior and data preservation](docs/updates.md).

## Develop

Start with [building](docs/building.md), [architecture](docs/architecture.md),
[contributing](CONTRIBUTING.md) and [security](SECURITY.md). The [release guide](docs/releasing.md)
covers source export, CI, signing, draft releases and updater asset naming.

The repository is licensed under MIT. Bundled third-party components keep their
own licenses and attribution; llama.cpp provenance and model terms are in the
source tree. No model weights, private library data or credentials are published.

This is a preview: test file operations on expendable copies first and keep your
normal backups. Performance varies with corpus, storage and network conditions;
Retriever does not claim a universal 4× speedup over Explorer or Finder.
