"""Build the release's standalone LGPL FFmpeg and stage pinned PDFium.

macos-arm64 builds on macOS; windows-x64 cross-compiles on Linux with MinGW.
Python is build tooling only. No system runtime or user data is modified.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

FFMPEG_SHA = "b6863adde98898f42602017462871b5f6333e65aec803fdd7a6308639c52edf3"
PDFIUM = {
    "windows-x64": ("pdfium-win-x64.tgz", "55329d5cb5de8a379a2fc563106492d7f385a1f795d18970922c71f708f9fbb4", "pdfium.dll"),
    "macos-arm64": ("pdfium-mac-arm64.tgz", "e214ee33f22b2204daa765a545aee1e425d88448e6154dac95c6a06206b7437f", "libpdfium.dylib"),
}

def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()

def fetch(url, path, expected):
    if not path.exists():
        temporary = path.with_suffix(path.suffix + ".partial")
        with urllib.request.urlopen(url, timeout=120) as response, temporary.open("wb") as out:
            shutil.copyfileobj(response, out)
        if digest(temporary) != expected:
            raise ValueError("Dependency checksum mismatch: " + path.name)
        temporary.replace(path)
    if digest(path) != expected:
        raise ValueError("Dependency checksum mismatch: " + path.name)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=PDFIUM)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    root = args.output.resolve(); root.mkdir(parents=True, exist_ok=True)
    source = root / "build"; source.mkdir(exist_ok=True)
    notices = root / "notices"; notices.mkdir(exist_ok=True)
    ffmpeg = source / "ffmpeg-8.1.1.tar.xz"
    fetch("https://ffmpeg.org/releases/ffmpeg-8.1.1.tar.xz", ffmpeg, FFMPEG_SHA)
    if not (source / "ffmpeg-8.1.1/configure").exists():
        with tarfile.open(ffmpeg) as tar:
            tar.extractall(source, filter="data")
    work = source / "ffmpeg-8.1.1"
    flags = ["--disable-autodetect", "--disable-network", "--disable-doc", "--disable-debug", "--disable-ffplay", "--disable-gpl", "--disable-nonfree", "--disable-shared", "--enable-static", "--disable-x86asm"]
    env = dict(os.environ)
    if args.platform == "windows-x64":
        flags += ["--target-os=mingw32", "--arch=x86_64", "--enable-cross-compile", "--cross-prefix=x86_64-w64-mingw32-", "--extra-ldflags=-static", "--pkg-config=false"]
    else:
        env["MACOSX_DEPLOYMENT_TARGET"] = "14.0"
        sdk = subprocess.check_output(["xcrun", "--sdk", "macosx", "--show-sdk-path"], text=True).strip()
        flags += ["--arch=arm64", "--target-os=darwin", "--cc=" + subprocess.check_output(["xcrun", "--find", "clang"], text=True).strip(), "--sysroot=" + sdk, "--extra-cflags=-mmacosx-version-min=14.0", "--extra-ldflags=-mmacosx-version-min=14.0"]
    with (root / "build.log").open("w") as log:
        subprocess.run(["sh", "configure", *flags], cwd=work, env=env, check=True, stdout=log, stderr=subprocess.STDOUT)
        subprocess.run(["make", "-j", str(min(4, os.cpu_count() or 2)), "ffmpeg", "ffprobe"], cwd=work, env=env, check=True, stdout=log, stderr=subprocess.STDOUT)
    suffix = ".exe" if args.platform == "windows-x64" else ""
    names = ["ffmpeg" + suffix, "ffprobe" + suffix]
    for name in names:
        shutil.copy2(work / name, root / name)
    ffnotice = notices / "ffmpeg"; ffnotice.mkdir(exist_ok=True)
    for name in ["COPYING.LGPLv2.1", "LICENSE.md"]:
        shutil.copy2(work / name, ffnotice / name)
    # Complete corresponding source and the exact recipe accompany each binary.
    shutil.copy2(ffmpeg, ffnotice / ffmpeg.name)
    shutil.copy2(__file__, ffnotice / "rebuild.py")
    (ffnotice / "configure-arguments.json").write_text(json.dumps(flags, indent=2) + "\n")
    filename, sha, binary = PDFIUM[args.platform]
    pdfarchive = source / filename
    fetch("https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7999/" + filename, pdfarchive, sha)
    pdf = source / "pdfium"; pdf.mkdir(exist_ok=True)
    with tarfile.open(pdfarchive) as tar:
        tar.extractall(pdf, filter="data")
    candidates = list(pdf.rglob(binary))
    if len(candidates) != 1:
        raise ValueError("Missing or ambiguous PDFium runtime")
    shutil.copy2(candidates[0], root / binary); names.append(binary)
    shutil.copytree(pdf / "licenses", notices / "pdfium", dirs_exist_ok=True)
    if args.platform == "windows-x64":
        mingw = notices / "mingw"; mingw.mkdir(exist_ok=True)
        for name in ["mingw-w64-common", "gcc-mingw-w64-x86-64-posix", "mingw-w64-x86-64-dev"]:
            copyright_file = Path("/usr/share/doc") / name / "copyright"
            if copyright_file.exists():
                shutil.copy2(copyright_file, mingw / (name + ".txt"))
        if not list(mingw.iterdir()):
            raise ValueError("MinGW runtime attribution is required")
    manifest = {"platform": args.platform, "ffmpeg": "8.1.1", "ffmpeg_license": "LGPL-2.1-or-later", "ffmpeg_source_sha256": FFMPEG_SHA, "pdfium": "153.0.7999.0", "files": [{"name": n, "bytes": (root/n).stat().st_size, "sha256": digest(root/n)} for n in names]}
    (root / "media-runtime.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"platform": args.platform, "files": len(names), "output": str(root)}))

if __name__ == "__main__":
    main()
