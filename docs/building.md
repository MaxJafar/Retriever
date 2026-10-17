# Building Retriever

The app runs Rust and native UI code. Python 3.11+ is used only for release tooling
and optional model conversion. It is not installed with the app.

## Requirements

- Rust 1.97.1 (the version used by CI), CMake 3.25+ and a C++17 compiler.
- Windows x64: Visual Studio 2022 C++ build tools, Windows SDK, .NET 10 SDK,
  PowerShell 7 and Inno Setup 6 for installers.
- Apple Silicon: macOS 14+, Xcode command-line tools, Swift and Python 3.11+.

The llama.cpp sources are vendored at the revision in
`retriever-rs/vendor/llama.cpp/RETRIEVER-PROVENANCE.txt`. `Cargo.lock` pins Rust
dependencies; use `--locked`. CUDA builds are optional: set
`RETRIEVER_EMBEDDING_CUDA=1`, `CUDA_PATH` and `RETRIEVER_CUDA_ARCHITECTURES` for the
target GPU. The portable public build uses optimized CPU kernels.

On Windows and Mac, point `TEMP`/`TMP`/`TMPDIR` at a new fixture directory outside
AppData and protected system folders. The crawler intentionally excludes those
locations. CI creates `artifacts/test-tmp` for this purpose.

```sh
cargo test --locked --release --manifest-path retriever-rs/Cargo.toml --workspace
cargo build --locked --release --manifest-path retriever-rs/Cargo.toml -p retriever -p retriever-agent -p retriever-updater
python -m unittest discover -s release -p 'test_*.py'
```

On Windows, run `./native/retriever-windows/build.ps1` in PowerShell. On macOS:

```sh
export RETRIEVER_ENGINE="$PWD/retriever-rs/target/release/retriever"
export RETRIEVER_UPDATER="$PWD/retriever-rs/target/release/retriever-updater"
sh native/retriever-macos/build.sh
```

The native shell starts the local coordinator. Use `RETRIEVER_STATE` and
`RETRIEVER_PORT` for isolated development; never run test indexing against your
production state. A CLI coordinator can also be started with:

```sh
retriever-rs/target/release/retriever --state /absolute/test-state serve --port 18781
```

## Native AI models

Model weights are separate from source and app updates. An existing installation
keeps its models. First-time semantic search requires an EmbeddingGemma GGUF and
the compatible Laya multilingual checkpoint, obtained under their upstream
licenses. Read `retriever-rs/model-notices` before redistribution. Conversion is
offline and needs the build dependencies documented at the top of
`retriever-rs/tools/prepare_native_models.py`.

```sh
python retriever-rs/tools/prepare_native_models.py --embedding /path/model.gguf --laya /path/checkpoint --output /path/native-bundle
retriever-rs/target/release/retriever --state /path/to/your/state install-models /path/native-bundle
```

The bundle includes per-file SHA-256 checks, ONNX Runtime and attribution. Install
it into the same state directory selected by the desktop. Do not overwrite a
different existing bundle. Filename and indexed-text search remain usable
without model weights; meaning search reports missing models clearly.

## Native media and installers

PDFium is downloaded from a pinned upstream release and verified by SHA-256.
FFmpeg 8.1.1 is built from pinned source with GPL and nonfree components disabled;
the source, build recipe and LGPL notices accompany its standalone executables.

```sh
# On macOS:
python release/media.py macos-arm64 artifacts/media
python release/build.py macos-arm64 --output dist --media artifacts/media

# On Linux with gcc-mingw-w64-x86-64 and make:
python release/media.py windows-x64 artifacts/media-windows
# Copy only media binaries, media-runtime.json and notices to the Windows builder.
# On Windows:
python release/build.py windows-x64 --output dist --media artifacts/media-windows --inno "C:/Program Files (x86)/Inno Setup 6/ISCC.exe"
```

The release workflow performs these steps on clean Windows and Mac runners.
Build products go under `artifacts/` or `dist/`; no state, model weights or
deployment configuration belongs in a package. See [releasing](releasing.md)
before distributing unsigned preview builds or configuring signing.
