"""Launch/search/checkpoint/relaunch an isolated library; never opens existing user state."""
import argparse
import hashlib
import json
from pathlib import Path
import socket
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request

def run(engine, output):
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ValueError("Smoke output must be empty; existing state is preserved")
    state = output / "state"
    subprocess.run([str(engine), "--state", str(state), "init"], check=True, capture_output=True)
    # A synthetic metadata/cache row tests persistence without a copied library.
    with sqlite3.connect(state / "retriever.sqlite") as db:
        db.execute("INSERT INTO workspace_meta(device,path,owner,tags,properties) VALUES (?,?,?,?,?)", ("fixture", "report.txt", "Example", '["keep"]', '{"priority":"high"}'))
        db.execute("INSERT INTO embedding_cache(fingerprint,content_hash,vector,dimensions) VALUES (?,?,?,?)", ("fixture", "keep", b"\0" * 16, 4))
    protected = {name: hashlib.sha256((state/name).read_bytes()).hexdigest() for name in ["api-token", "config.json", "update-token"]}
    token = (state / "api-token").read_text().strip()
    update_token = (state / "update-token").read_text().strip()
    with socket.socket() as available:
        available.bind(("127.0.0.1", 0)); port = available.getsockname()[1]
    endpoint = f"http://127.0.0.1:{port}/api/"
    def request(route, body=None, local=False):
        headers = {"Authorization": "Bearer " + token, "Content-Type": "application/json"}
        if local: headers["X-Retriever-Update-Token"] = update_token
        call = urllib.request.Request(endpoint + route, data=json.dumps(body).encode() if body is not None else None, headers=headers)
        with urllib.request.urlopen(call, timeout=5) as reply:
            return json.load(reply)
    checks = []
    for cycle in range(2):
        with (output / f"engine-{cycle}.log").open("wb") as log:
            process = subprocess.Popen([str(engine), "--state", str(state), "serve", "--port", str(port)], stdout=log, stderr=log)
            try:
                for _ in range(100):
                    try: status = request("status"); break
                    except (urllib.error.URLError, ConnectionError):
                        if process.poll() is not None: raise RuntimeError("Engine exited before status")
                        time.sleep(.1)
                else: raise RuntimeError("Engine did not start")
                assert status["update_protocol"] == 1 and status["runtime"] == "rust"
                request("search?q=README&mode=exact")
                try:
                    request("runtime/shutdown", {"expected_version": status["version"]})
                    raise AssertionError("Shared token was allowed to stop the engine")
                except urllib.error.HTTPError as error:
                    assert error.code == 403
                reply = request("runtime/shutdown", {"expected_version": status["version"]}, local=True)
                assert reply["checkpointed"]
                assert process.wait(timeout=15) == 0
                checks.append({"cycle": cycle, "launch": True, "search": True, "checkpoint_shutdown": True})
            finally:
                if process.poll() is None:
                    process.terminate(); process.wait(timeout=10)
    for name, digest in protected.items():
        assert hashlib.sha256((state/name).read_bytes()).hexdigest() == digest
    with sqlite3.connect(state / "retriever.sqlite") as db:
        assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert db.execute("SELECT owner,tags,properties FROM workspace_meta WHERE device='fixture'").fetchone() == ("Example", '["keep"]', '{"priority":"high"}')
        assert db.execute("SELECT vector FROM embedding_cache WHERE fingerprint='fixture'").fetchone()[0] == b"\0" * 16
    report = {"passed": True, "fixture_only": True, "checks": checks, "metadata_cache_and_credentials_preserved": True}
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    run(args.engine.resolve(strict=True), args.output.resolve())
