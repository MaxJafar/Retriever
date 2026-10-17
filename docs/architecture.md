# Architecture and boundaries

`retriever-rs` contains a Rust coordinator, SSH node agent, native model bridge
and update helper. `native/retriever-windows` uses WinUI 3; `native/retriever-macos`
uses SwiftUI and AppKit. The embedded web client supports the same coordinator.

The coordinator owns configuration, indexed text, vectors, cached model results,
metadata and operation receipts in its state directory. Sources are selected
explicitly. Browsing, indexing, copy/move previews and file transfers share path
validation. Copies use native local operations where possible; remote transfers
can adapt compression to the content. Compression is lossless. No universal
speed multiplier against Explorer or Finder is claimed.

EmbeddingGemma and Laya execute locally in process through llama.cpp and ONNX
Runtime. Model weights are installed separately and stay outside app updates.
The public source contains the runtime integration and conversion tools, not
weights or private indexes. Multi-device mode can share one coordinator's index
through an authenticated connection instead of duplicating it on every machine.

Disk Capacity measures the selected filesystem. Disk Health remains a synthetic
fixture viewer: the production collector is blocked, including SMART access.
Do not change this boundary while fixing unrelated disk or update features.

Updates run through a small credential-free helper. Its network traffic cannot
reuse a coordinator's Authorization header. Local shutdown authorization is
separate from shared-library authorization. See [updates](updates.md).

The public export is an allowlist, not a copy of a developer's workspace. Models,
SSH configuration, credentials, personal documents, SQLite files, caches,
acceptance reports and historical builds are deliberately excluded.
