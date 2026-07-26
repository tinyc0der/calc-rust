#!/usr/bin/env bash
# Prove the project links no C or C++ library.
#
# This is the project's hardest constraint and the one most easily lost by
# accident: `astro-float`'s *default* feature set pulls `random` -> `rand` ->
# `getrandom` -> `libc`, so a `cargo add astro-float`, a dependency-bot
# manifest rewrite, or a hand-edit that drops `default-features = false` from
# crates/qalc-num/Cargo.toml silently breaks it. Nothing else in the build
# would complain.
#
# Two gates. The first reads resolved metadata and is cheap; the second reads
# the source of every crate actually in the graph and is thorough. Both use
# --locked so an un-reviewed lockfile change fails here rather than quietly
# re-resolving.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Gate A: resolved dependency metadata"
cargo metadata --format-version 1 --locked | python3 -c '
import json, sys, re
packages = [p for p in json.load(sys.stdin)["packages"] if p["source"]]
banned = re.compile(
    r"(-sys$|^libc$|^cc$|^cmake$|^pkg-config$|^bindgen$|^rug$"
    r"|^gmp-mpfr-sys$|^openssl$|^native-tls$)"
)
failures = [
    f"banned crate: {p['name']} {p['version']}"
    for p in packages
    if banned.search(p["name"])
]
# A `links` key declares that the crate owns a native library.
failures += [
    f"declares a native library: {p['name']} -> {p['links']}"
    for p in packages
    if p.get("links")
]
print(f"  {len(packages)} external crates in the graph")
sys.exit("\n".join(failures) or 0)
'

echo "== Gate B: source scan of every crate in the graph"
cargo metadata --format-version 1 --locked | python3 -c '
import json, sys, re, pathlib
packages = [p for p in json.load(sys.stdin)["packages"] if p["source"]]
links = re.compile(r"rustc-link-lib|rustc-link-search|cc::Build|pkg_config|cmake::")
ffi = re.compile(r"extern\s+\"C\"\s*\{|#\[link[(_]")
failures = []
for package in packages:
    root = pathlib.Path(package["manifest_path"]).parent
    for source in root.rglob("*.rs"):
        text = source.read_text(errors="ignore")
        if source.name == "build.rs" and links.search(text):
            failures.append(f"build.rs links a C library: {source}")
        # syn is a Rust *parser*: its only `extern "C"` hits are doc comments
        # describing the syntax node.
        if package["name"] != "syn" and ffi.search(text):
            failures.append(f"FFI declaration: {source}")
print("  no FFI, no native linkage")
sys.exit("\n".join(failures) or 0)
'

echo "== Gate C: the workspace itself"
if grep -rn --include=*.rs -E 'extern\s+"C"|#\[link[(_]' crates/; then
    echo "FFI declaration in workspace source" >&2
    exit 1
fi
echo "  clean"

echo
echo "PURE RUST OK"
