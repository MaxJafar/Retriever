"""Collect locked Rust and native runtime attribution, without build-host paths."""
import json
from pathlib import Path
import shutil
import subprocess

def collect(root, output, platform):
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root / "LICENSE", output / "RETRIEVER-LICENSE.txt")
    vendor = root / "retriever-rs/vendor/llama.cpp"
    for name in ["LICENSE", "RETRIEVER-PROVENANCE.txt"]:
        shutil.copy2(vendor / name, output / ("LLAMA-CPP-" + name + ".txt"))
    for path in vendor.rglob("*"):
        if path.is_file() and path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")):
            target_notice = output / "llama.cpp" / path.relative_to(vendor)
            target_notice.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target_notice)
    target = {"windows-x64": "x86_64-pc-windows-msvc", "macos-arm64": "aarch64-apple-darwin"}[platform]
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--filter-platform", target, "--manifest-path", str(root / "retriever-rs/Cargo.toml"), "--format-version", "1"]))
    inventory = []
    for crate in metadata["packages"]:
        inventory.append({key: crate.get(key) for key in ["name", "version", "license", "repository"]})
        for path in Path(crate["manifest_path"]).parent.iterdir():
            if path.is_file() and path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")):
                dest = output / "rust" / (crate["name"] + "-" + crate["version"])
                dest.mkdir(parents=True, exist_ok=True); shutil.copy2(path, dest / path.name)
    (output / "rust-dependencies.json").write_text(json.dumps(inventory, indent=2) + "\n")
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    std = output / "rust-standard-library"; std.mkdir(exist_ok=True)
    attribution = sysroot / "share/doc/rust"
    if not (attribution / "COPYRIGHT-library.html").is_file():
        version = subprocess.check_output(["rustc", "--version"], text=True).split()[1]
        attribution = root / "release/rust-notices" / version
    shutil.copy2(attribution / "COPYRIGHT-library.html", std)
    shutil.copytree(attribution / "licenses", std / "licenses", dirs_exist_ok=True)
