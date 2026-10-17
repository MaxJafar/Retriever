"""Package native Retriever. Requires a verified media build; never bundles state or weights."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib
import xml.etree.ElementTree as ET
from notices import collect
from check_media import check as check_media

def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=["windows-x64", "macos-arm64"])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--media", type=Path, required=True)
    parser.add_argument("--engine-dir", type=Path)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--inno", default="ISCC.exe")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "retriever-rs/Cargo.toml").read_text())["workspace"]["package"]["version"]
    if ET.parse(root / "native/retriever-windows/Retriever.csproj").findtext("PropertyGroup/Version") != version:
        raise ValueError("Native and engine versions differ")
    output = args.output.resolve(); output.mkdir(parents=True, exist_ok=True)
    package = output / (f"Retriever-{version}-" + ("windows-x64-setup.exe" if args.platform == "windows-x64" else "macos-arm64.dmg"))
    if package.exists():
        raise ValueError("Retaining existing release. Choose a new version/output.")
    media = args.media.resolve()
    manifest = json.loads((media / "media-runtime.json").read_text(encoding="utf-8-sig"))
    if manifest.get("platform") != args.platform or manifest.get("ffmpeg_license") != "LGPL-2.1-or-later":
        raise ValueError("Use release/media.py to prepare redistributable decoders")
    required = {"ffmpeg.exe", "ffprobe.exe", "pdfium.dll"} if args.platform == "windows-x64" else {"ffmpeg", "ffprobe", "libpdfium.dylib"}
    if {row["name"] for row in manifest["files"]} != required:
        raise ValueError("Incomplete media runtime")
    for row in manifest["files"]:
        if sha(media / row["name"]) != row["sha256"]:
            raise ValueError("Media checksum mismatch")
    if not args.skip_build:
        subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path", str(root / "retriever-rs/Cargo.toml"), "-p", "retriever", "-p", "retriever-agent", "-p", "retriever-updater"], check=True)
    engines = (args.engine_dir or Path(os.environ.get("CARGO_TARGET_DIR", root / "retriever-rs/target")) / "release").resolve()
    suffix = ".exe" if args.platform == "windows-x64" else ""
    for name in ["retriever", "retriever-agent", "retriever-updater"]:
        binary = engines / (name + suffix)
        if not binary.is_file():
            raise ValueError("Missing release binary: " + name)
    for name in ["retriever", "retriever-updater"]:
        if subprocess.check_output([str(engines / (name + suffix)), "--version"], text=True).strip().split()[-1] != version:
            raise ValueError("Wrong release version: " + name)
    if args.platform == "windows-x64":
        payload = output / "payload-windows-x64"
        subprocess.run(["pwsh", "-NoProfile", "-File", str(root / "native/retriever-windows/build.ps1"), "-OutputDirectory", str(payload), "-EnginePath", str(engines / "retriever.exe")], check=True)
        resources = payload
        subprocess.run(["pwsh", "-NoProfile", "-File", str(root / "release/windows-runtime.ps1"), "-Payload", str(payload)], check=True)
    else:
        payload = output / "dmg-content" / "Retriever.app"
        env = dict(os.environ, RETRIEVER_BUILD_OUT=str(payload), RETRIEVER_ENGINE=str(engines / "retriever"), RETRIEVER_UPDATER=str(engines / "retriever-updater"))
        subprocess.run(["sh", str(root / "native/retriever-macos/build.sh")], env=env, check=True)
        resources = payload / "Contents/Resources"
    engine = resources / "engine"; engine.mkdir(exist_ok=True)
    for name in ["retriever", "retriever-agent", "retriever-updater"]:
        shutil.copy2(engines / (name + suffix), engine / (name + suffix))
    for name in required:
        shutil.copy2(media / name, engine / name)
    shutil.copy2(media / "media-runtime.json", engine)
    check_media(engine, args.platform, output / "media-check")
    collect(root, resources / "notices", args.platform)
    shutil.copytree(media / "notices", resources / "notices", dirs_exist_ok=True)
    shutil.copy2(root / "release/PREVIEW-NOTICE.txt", resources)
    shutil.copy2(root / "docs/updates.md", resources)
    def inventory():
        files = [{"path": p.relative_to(resources).as_posix(), "bytes": p.stat().st_size, "sha256": sha(p)} for p in sorted(resources.rglob("*")) if p.is_file() and p.name != "release.json"]
        (resources / "release.json").write_text(json.dumps({"product": "Retriever", "version": version, "platform": args.platform, "models_included": False, "health_collection": "fixture-only", "files": files}, indent=2) + "\n")
    inventory()
    if args.platform == "windows-x64":
        subprocess.run([args.inno, "/DPayload=" + str(payload), "/DOutput=" + str(output), "/DVersion=" + version, str(root / "native/retriever-windows/installer.iss")], check=True)
    else:
        identity = os.environ.get("RETRIEVER_SIGN_IDENTITY", "-")
        # Sign nested executables first; release signing uses Developer ID + hardened runtime.
        for binary in engine.iterdir():
            if binary.is_file() and binary.suffix != ".json":
                command = ["codesign", "--force", "--sign", identity]
                if identity != "-":
                    command += ["--options", "runtime", "--timestamp"]
                    if binary.name == "retriever": command += ["--entitlements", str(root / "release/macos-entitlements.plist")]
                subprocess.run(command + [str(binary)], check=True)
        inventory()  # Nested code signing changes executable bytes.
        command = ["codesign", "--force", "--sign", identity]
        if identity != "-": command += ["--options", "runtime", "--timestamp"]
        subprocess.run(command + [str(payload)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(payload)], check=True)
        applications = payload.parent / "Applications"
        if not applications.exists(): applications.symlink_to("/Applications", target_is_directory=True)
        subprocess.run(["hdiutil", "create", "-volname", "Retriever", "-srcfolder", str(payload.parent), "-format", "UDZO", str(package)], check=True)
        subprocess.run(["hdiutil", "verify", str(package)], check=True)
        profile = os.environ.get("RETRIEVER_NOTARY_PROFILE")
        if profile:
            subprocess.run(["xcrun", "notarytool", "submit", str(package), "--keychain-profile", profile, "--wait"], check=True)
            subprocess.run(["xcrun", "stapler", "staple", str(package)], check=True)
    (output / (package.name + ".sha256")).write_text(sha(package) + "  " + package.name + "\n")
    print(json.dumps({"package": str(package), "bytes": package.stat().st_size, "sha256": sha(package)}))

if __name__ == "__main__":
    main()
