#!/usr/bin/env python3
"""Stage this fork and the forks it pins as `-kora` crates that crates.io accepts.

Usage: stage.py <iced-checkout> <out-dir> [--local]

Fetches the wgpu, winit, cosmic-text, cryoglyph and softbuffer forks at the commits this
Cargo.toml pins, copies every fork to <out-dir>/<fork>, renames the crates it
publishes to `<name>-kora`, and turns git dependencies on the other forks into
crates.io dependencies on the renamed crates. Code keeps its crate names, so
`use iced::...` still works. With --local, each staged fork also patches
crates.io with the staged forks before it, so the chain builds before anything
is published.
"""

import json
import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

import tomlkit

SUFFIX = "-kora"

# Fork -> (version to publish, crates it publishes), in publishing order.
# Bump a fork's version to publish its pinned commit again.
FORKS = {
    "cosmic-text": ("0.18.1", ["cosmic-text"]),
    "wgpu": ("28.0.1", [
        "wgpu", "wgpu-core", "wgpu-hal", "wgpu-types",
        "wgpu-core-deps-apple", "wgpu-core-deps-emscripten",
        "wgpu-core-deps-wasm", "wgpu-core-deps-windows-linux-android",
    ]),
    "winit": ("0.30.14", ["winit"]),
    "cryoglyph": ("0.1.1", ["cryoglyph"]),
    "softbuffer": ("0.4.8", ["softbuffer"]),
    "iced": ("0.15.1", [
        "iced", "iced_automation", "iced_beacon", "iced_core", "iced_debug",
        "iced_devtools", "iced_futures", "iced_graphics", "iced_highlighter",
        "iced_program", "iced_renderer", "iced_runtime", "iced_selector",
        "iced_test", "iced_tester", "iced_tiny_skia", "iced_wgpu",
        "iced_widget", "iced_winit",
    ]),
}

DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def kora_name(name):
    return name.replace("_", "-") + SUFFIX


# Every renamed crate: original name -> (fork, new name, version).
RENAMED = {
    crate: (fork, kora_name(crate), version)
    for fork, (version, crates) in FORKS.items()
    for crate in crates
}


def git_url_fork(url):
    for fork in FORKS:
        if url.rstrip("/").removesuffix(".git").endswith(f"playtron-os/{fork}"):
            return fork
    return None


def resolve(fork, ref):
    """Full commit hash of a branch, tag or (short) hash in a fork, from the GitHub API."""
    request = urllib.request.Request(f"https://api.github.com/repos/playtron-os/{fork}/commits/{ref}")
    if os.environ.get("GITHUB_TOKEN"):
        request.add_header("Authorization", f"Bearer {os.environ['GITHUB_TOKEN']}")
    with urllib.request.urlopen(request) as response:
        return json.load(response)["sha"]


def git_ref(dep):
    return dep.get("rev") or dep.get("tag") or dep.get("branch") or "HEAD"


def fetch_pinned(iced, src):
    """Fetch every fork iced pins, and return fork -> commit."""
    pins = {}
    for key, dep in tomlkit.parse((iced / "Cargo.toml").read_text())["workspace"]["dependencies"].items():
        fork = git_url_fork(dep.get("git", "")) if hasattr(dep, "get") else None
        if fork:
            pins[fork] = resolve(fork, git_ref(dep))
    for fork, commit in pins.items():
        dest = src / fork
        shutil.rmtree(dest, ignore_errors=True)
        subprocess.run(["git", "init", "-q", str(dest)], check=True)
        subprocess.run(["git", "-C", str(dest), "fetch", "-q", "--depth", "1",
                        f"https://github.com/playtron-os/{fork}.git", commit], check=True)
        subprocess.run(["git", "-C", str(dest), "checkout", "-q", "FETCH_HEAD"], check=True)
        print(f"{fork} {commit}", flush=True)
    return pins


def rewrite_dep(table, key, fork, pins):
    """Point one dependency entry at the renamed crate, if it is one of ours."""
    dep = table[key]
    # Registry dependencies, such as wgpu-hal's dev-dependency on winit, stay upstream.
    if not hasattr(dep, "get") or dep.get("workspace") is True:
        return
    name = dep.get("package", key)
    if name not in RENAMED:
        return
    dep_fork, new_name, version = RENAMED[name]
    entry = {k: v.unwrap() if hasattr(v, "unwrap") else v for k, v in dep.items()}
    if "git" in entry:
        if git_url_fork(entry["git"]) != dep_fork:
            return
        if resolve(dep_fork, git_ref(entry)) != pins[dep_fork]:
            raise SystemExit(f"{fork} pins {dep_fork} at {git_ref(entry)}, iced at {pins[dep_fork]}")
        for k in ("git", "rev", "branch", "tag"):
            entry.pop(k, None)
        entry["version"] = version
    elif "path" in entry and dep_fork == fork:
        if "version" in entry:
            entry["version"] = version
    else:
        return
    entry["package"] = new_name

    # tomlkit drops a comma when it adds a key to a multi-line inline table,
    # so build a fresh one. A [dependencies.<name>] table edits cleanly.
    if isinstance(dep, tomlkit.items.Table):
        dep.clear()
        dep.update(entry)
        return
    inline = tomlkit.inline_table()
    inline.update(entry)
    table[key] = inline


def dep_tables(doc):
    for table in DEP_TABLES:
        if table in doc:
            yield doc[table]
    for target in doc.get("target", {}).values():
        for table in DEP_TABLES:
            if table in target:
                yield target[table]
    workspace = doc.get("workspace", {})
    if "dependencies" in workspace:
        yield workspace["dependencies"]


def rewrite_manifest(path, fork, pins):
    version = FORKS[fork][0]
    doc = tomlkit.parse(path.read_text())
    for table in dep_tables(doc):
        for key in list(table.keys()):
            rewrite_dep(table, key, fork, pins)
    # A patch that points at a renamed sibling no longer matches its name.
    for source in doc.get("patch", {}).values():
        for key in list(source.keys()):
            if key in RENAMED and RENAMED[key][0] == fork:
                del source[key]

    if "version" in doc.get("workspace", {}).get("package", {}):
        doc["workspace"]["package"]["version"] = version

    package = doc.get("package")
    name = package.get("name") if package else None
    if name in RENAMED and RENAMED[name][0] == fork:
        package["name"] = kora_name(name)
        if not hasattr(package.get("version"), "get"):
            package["version"] = version
        package["repository"] = f"https://github.com/playtron-os/{fork}"
        package.pop("documentation", None)
        package.pop("homepage", None)
        if "description" in package:
            package["description"] = f"{package['description']} (Playtron's Kora fork of {name})"
        # Keep the library name, so the crate's own tests and examples still compile.
        lib = doc.setdefault("lib", tomlkit.table())
        if "name" not in lib:
            lib["name"] = name.replace("-", "_")

    path.write_text(tomlkit.dumps(doc))


def add_local_patches(out, fork):
    """Patch crates.io with the staged forks before this one, so nothing has to be published yet."""
    root = out / fork / "Cargo.toml"
    doc = tomlkit.parse(root.read_text())
    patch = doc.setdefault("patch", tomlkit.table()).setdefault("crates-io", tomlkit.table())
    earlier = list(FORKS)[: list(FORKS).index(fork)]
    for dep_fork in earlier:
        for crate in FORKS[dep_fork][1]:
            patch[kora_name(crate)] = {"path": str(find_manifest(out / dep_fork, kora_name(crate)).parent)}
    root.write_text(tomlkit.dumps(doc))


def find_manifest(root, package_name):
    for manifest in root.rglob("Cargo.toml"):
        if "target" in manifest.parts:
            continue
        if tomlkit.parse(manifest.read_text()).get("package", {}).get("name") == package_name:
            return manifest
    raise SystemExit(f"no manifest for {package_name} under {root}")


def main():
    iced, out = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    local = "--local" in sys.argv[3:]
    if out.is_relative_to(iced):
        raise SystemExit("stage outside the iced checkout, or it copies itself")
    sources ={fork: out / "src" / fork for fork in FORKS}
    sources["iced"] = iced
    pins = fetch_pinned(iced, out / "src")
    for fork in FORKS:
        dest = out / fork
        shutil.rmtree(dest, ignore_errors=True)
        # Leave this tooling out of the published iced crate.
        shutil.copytree(sources[fork], dest, ignore=shutil.ignore_patterns(".git", "target", "kora"))
        for manifest in dest.rglob("Cargo.toml"):
            rewrite_manifest(manifest, fork, pins)
        (dest / "Cargo.lock").unlink(missing_ok=True)
    if local:
        for fork in FORKS:
            add_local_patches(out, fork)


if __name__ == "__main__":
    main()
