# Releasing Retriever

## First publication

1. Set `release/repository.json` to the public `owner/repository` that will own
   updates. The prepared default is `MaxJafar/Retriever`; forks must change it.
2. Run `python -m unittest discover -s release -p 'test_*.py'` and export only the
   reviewed source: `python release/export.py dist/Retriever-source.tar.gz --directory dist/repository`.
   Do not initialize Git or push a mixed private development workspace wholesale.
3. Review the export and manifest, then create the public MIT repository from that
   directory. Enable private vulnerability reporting, branch protection and
   protected release tags. Review generated assets before publishing a release.
4. Run the native CI on Windows and Apple Silicon. Validate launch, read-only
   search, file-operation guards, update controls and state preservation. These
   checks do not require a personal library to be uploaded to GitHub.

## Publish a version

Keep `retriever-rs/Cargo.toml` and `native/retriever-windows/Retriever.csproj`
versions equal; refresh Cargo.lock with Cargo and update CHANGELOG.md. The Mac
bundle reads the Rust workspace version. Create tag `vVERSION` after checks pass.

The **Draft native release** workflow verifies the tag/repository, exports source,
builds pinned native decoders, runs Rust tests, compiles both clients and produces:

- `Retriever-VERSION-windows-x64-setup.exe`
- `Retriever-VERSION-macos-arm64.dmg`
- `Retriever-source.tar.gz`, plus SHA-256 files

Only the final job has `contents: write`; it creates a **draft prerelease**.
Review the notes, signatures, checksums and both installed applications. Publish
that completed draft to make it discoverable by installed clients. The updater
requires GitHub's asset `digest` field; never publish an incomplete asset set or
replace installer bytes under an already published version. Advance the version
for fixes. Manual workflow dispatch on a branch builds and uploads review
artifacts without creating a release; dispatch on a release tag also creates
the draft. This permits checking clean-runner packaging before tagging.

A stable release needs signing and notarization. The supplied hosted workflow is
for preview builds and intentionally does not import signing secrets or publish
on its own. Adapt it to your protected signing environment before stable release.

## Signing

Windows: sign the app's executables/DLLs and final Inno Setup installer with your
publisher certificate and an RFC 3161 timestamp. Configure Inno Setup's signed
uninstaller support in the signing environment. Verify with `signtool verify /pa`.
No certificate, private key or signing password belongs in this repository.

macOS: set `RETRIEVER_SIGN_IDENTITY` to an installed Developer ID Application
identity, and `RETRIEVER_NOTARY_PROFILE` to an existing notarytool keychain profile
before `release/build.py`. The builder signs nested executables, enables hardened
runtime, submits the DMG for notarization and staples the ticket. The engine's
library-validation exception permits the integrity-checked optional ONNX model
runtime in app data. Verify `codesign --verify --deep --strict`, `spctl --assess`
and stapler validation on the final artifact. Without these settings, preview
builds are ad-hoc signed and are not notarized.

## Dependencies and models

Use `release/media.py` for redistributable media runtimes. It pins PDFium by hash
and builds FFmpeg with GPL/nonfree components disabled. LGPL source, the actual
build recipe and notices accompany the separate decoder executable. The package
also collects locked Rust, C++/.NET and platform notices. Review attribution when
updating dependencies. Model bundles are separate, verified assets under their
upstream terms, not an implicit part of the MIT source or updater payload.

## Data and rollback

Do not package `coordinator.json`, `installation.json`, state, models, private
keys or acceptance reports. Users retain their state in the existing historical
`Retriever Candidate` app-data directory. The desktop persists custom deployment
settings outside the executable; updates stop only the local idle engine and
checkpoint before handing off. A client using another machine's index must leave
that remote coordinator running.

Keep the previous app available for rollback. Never erase state, caches or model
weights when uninstalling or switching versions. Schema changes need an explicit
compatibility plan; do not silently run older engines against a newer schema.
See [the updater contract](updates.md) for the first upgrade from pre-updater
builds and its manual restart requirement.
