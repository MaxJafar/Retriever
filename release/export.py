"""Export only reviewed public source. Never export a development workspace wholesale."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile

ROOT_FILES = ["README.md", "LICENSE", "CHANGELOG.md", "CONTRIBUTING.md", "SECURITY.md", ".gitignore", ".gitattributes"]
EXPLICIT = [
    "docs/building.md", "docs/releasing.md", "docs/updates.md", "docs/architecture.md",
    ".github/workflows/native.yml", ".github/workflows/release.yml", ".github/dependabot.yml",
    ".github/ISSUE_TEMPLATE/bug_report.yml", ".github/ISSUE_TEMPLATE/feature_request.yml",
    "release/export.py", "release/smoke.py", "release/build.py", "release/notices.py", "release/media.py", "release/windows-runtime.ps1",
    "release/repository.json", "release/PREVIEW-NOTICE.txt", "release/validate.py", "release/test_release.py", "release/macos-entitlements.plist",
    "retriever-rs/Cargo.toml", "retriever-rs/Cargo.lock",
    "retriever-rs/crates/embedding-native/build.rs", "retriever-rs/crates/embedding-native/bridge.cpp",
    "retriever-rs/tools/prepare_native_models.py", "retriever-rs/tools/stage_native_models.py",
    "retriever-rs/tests/fixtures/drive-health.json", "retriever-rs/tests/fixtures/disk-capacity.json",
]
PATTERNS = [
    "release/rust-notices/**/*",
    "retriever-rs/crates/*/Cargo.toml", "retriever-rs/crates/*/src/**/*.rs", "retriever-rs/crates/*/src/*.sql",
    "retriever-rs/crates/*/tests/*.rs", "retriever-rs/crates/*/examples/*.rs",
    "retriever-rs/web/*.html", "retriever-rs/web/*.css", "retriever-rs/web/*.js", "retriever-rs/web/*.png",
    "retriever-rs/model-notices/*.txt", "native/retriever-windows/*.cs", "native/retriever-windows/*.xaml",
    "native/retriever-windows/*.csproj", "native/retriever-windows/*.manifest", "native/retriever-windows/*.iss",
    "native/retriever-windows/build.ps1", "native/retriever-windows/Retriever.png", "native/retriever-windows/Retriever.ico",
    "native/retriever-macos/*.swift", "native/retriever-macos/build.sh", "native/retriever-macos/Retriever.png",
    "retriever-rs/vendor/llama.cpp/**/*",
]
BLOCKED_PARTS = {"target", ".git", ".ssh", "node_modules", "bin", "obj", "build", "publish", "__pycache__", "laya-service"}
# Match concrete secret material and deployment locations, not references to token files.
PRIVATE = re.compile(rb"(?:-----BEGIN (?:OPENSSH|RSA|EC|DSA|PRIVATE) (?:PRIVATE )?KEY-----\r?\n[A-Za-z0-9+/=]{32,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|[A-Z]:[/\\]+Users[/\\]+(?!Example[/\\]|A[/\\]|Test User[/\\]|Public[/\\]|you[/\\])[A-Za-z0-9._-]+[/\\]|/Users/(?!example/|test/|Shared/|name/|a/)[A-Za-z][A-Za-z0-9._-]*/|192\.168\.\d+\.\d+)", re.I)

def entries(root):
    files = {root / name for name in ROOT_FILES + EXPLICIT}
    for pattern in PATTERNS:
        files.update(p for p in root.glob(pattern) if p.is_file())
    result = []
    for path in sorted(files):
        relative = path.relative_to(root)
        if set(relative.parts) & BLOCKED_PARTS:
            continue
        if not path.is_file() or path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
            raise ValueError("Missing or linked source entry: " + relative.as_posix())
        data = path.read_bytes()
        try:
            text = data.decode("utf-8") if b"\0" not in data else None
        except UnicodeDecodeError:
            text = None
        if text is not None:
            data = data.replace(b"\r\n", b"\n")
            if PRIVATE.search(data):
                raise ValueError("Private content requires review: " + relative.as_posix())
        result.append((relative.as_posix(), data, 0o755 if path.suffix == ".sh" else 0o644))
    return result

def export(root, archive, directory=None):
    rows = list(entries(root))
    manifest = {"format": 1, "files": [{"path": name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()} for name, data, _ in rows]}
    rows.append(("retriever-source-manifest.json", (json.dumps(manifest, indent=2) + "\n").encode(), 0o644))
    if directory is not None:
        directory.mkdir(parents=True, exist_ok=True)
        if any(directory.iterdir()):
            raise ValueError("Export directory must be empty; existing files are preserved")
        for name, data, mode in rows:
            target = directory / name; target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data); target.chmod(mode)
    archive.parent.mkdir(parents=True, exist_ok=True)
    with archive.open("xb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w") as tar:
        for name, data, mode in rows:
            info = tarfile.TarInfo(name); info.size = len(data); info.mode = mode; info.mtime = 0
            tar.addfile(info, io.BytesIO(data))
    verify(archive)
    return {"files": len(manifest["files"]), "bytes": archive.stat().st_size, "sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}

def verify(archive):
    with tarfile.open(archive) as tar:
        members = tar.getmembers()
        names = [m.name for m in members]
        if len(names) != len(set(names)) or any(not m.isfile() or Path(m.name).is_absolute() or ".." in Path(m.name).parts for m in members):
            raise ValueError("Invalid source archive entries")
        manifest = json.load(tar.extractfile("retriever-source-manifest.json"))
        if set(names) != {r["path"] for r in manifest["files"]} | {"retriever-source-manifest.json"}:
            raise ValueError("Source manifest does not cover the archive")
        for row in manifest["files"]:
            data = tar.extractfile(row["path"]).read()
            if len(data) != row["bytes"] or hashlib.sha256(data).hexdigest() != row["sha256"]:
                raise ValueError("Source checksum mismatch")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--directory", type=Path)
    args = parser.parse_args()
    print(json.dumps(export(Path(__file__).resolve().parents[1], args.archive, args.directory)))
