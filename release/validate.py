"""Fail before packaging when versions, repository identity or release assets disagree."""
import argparse
import json
from pathlib import Path
import re
import tomllib
import xml.etree.ElementTree as ET

def validate(root, tag=None, repository=None):
    version = tomllib.loads((root / "retriever-rs/Cargo.toml").read_text())["workspace"]["package"]["version"]
    if ET.parse(root / "native/retriever-windows/Retriever.csproj").findtext("PropertyGroup/Version") != version:
        raise ValueError("Native and Rust versions disagree")
    repo = json.loads((root / "release/repository.json").read_text())["repository"]
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}", repo):
        raise ValueError("Invalid repository")
    if tag is not None and tag != "v" + version:
        raise ValueError("Tag must be v" + version)
    if repository is not None and repository.lower() != repo.lower():
        raise ValueError("Configure release/repository.json for this repository before releasing")
    return {"version": version, "repository": repo, "preview": "-" in version}

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag"); parser.add_argument("--repository")
    args = parser.parse_args()
    print(json.dumps(validate(Path(__file__).resolve().parents[1], args.tag, args.repository)))
