#!/bin/sh
# Every Tauri package on the npm side has to be on the same major.minor as its
# crate on the Rust side, or `tauri build` refuses -- and that check only runs in
# the release build, so a mismatch got past every pull request check and failed
# v0.13.1 on Windows after the tag was cut. This is the same check, from the two
# lockfiles, cheap enough to run on every change.
#
# Pairs are found by name: `@tauri-apps/api` is the `tauri` crate, and
# `@tauri-apps/plugin-x` is `tauri-plugin-x`. The CLI is not a pair.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
npm_lock="$root/apps/desktop/package-lock.json"
cargo_lock="$root/apps/desktop/src-tauri/Cargo.lock"

python3 - "$npm_lock" "$cargo_lock" <<'PY'
import json, re, sys

npm = json.load(open(sys.argv[1]))["packages"]
cargo = {}
for block in open(sys.argv[2]).read().split("[[package]]"):
    name = re.search(r'^name = "([^"]+)"', block, re.M)
    version = re.search(r'^version = "([^"]+)"', block, re.M)
    if name and version:
        cargo.setdefault(name.group(1), set()).add(version.group(1))

def minor(v):
    return ".".join(v.split(".")[:2])

bad = []
for path, meta in npm.items():
    m = re.fullmatch(r"node_modules/@tauri-apps/(api|plugin-[a-z-]+)", path)
    if not m:
        continue
    crate = "tauri" if m.group(1) == "api" else "tauri-" + m.group(1)
    for version in cargo.get(crate, ()):
        if minor(version) != minor(meta["version"]):
            bad.append(f"{crate} {version} : @tauri-apps/{m.group(1)} {meta['version']}")

if bad:
    print("::error::Tauri npm packages and crates disagree on major.minor:")
    print("\n".join(bad))
    sys.exit(1)
print("tauri npm packages and crates agree")
PY
