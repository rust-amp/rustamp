#!/usr/bin/env python3
"""Offline metadata guard, not legal advice or a full SPDX implementation."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


REQUIRED = {
    "rustamp-proto", "rustamp-plugin-abi", "rustamp-ipc",
    "rustamp-plughost", "rustamp-mediahost",
}
TARGETS = ("x86_64-pc-windows-msvc", "i686-pc-windows-msvc")
# All identifiers in every branch must be approved, even in an OR expression.
# Unknown identifiers/exceptions need review, not an automatic whitelist update.
PERMISSIVE = {
    "0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "MIT",
    "Unlicense", "Unicode-3.0", "Zlib",
}
BEGIN = "<!-- BEGIN CARGO LICENSE INVENTORY -->"
END = "<!-- END CARGO LICENSE INVENTORY -->"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def permissive(expression):
    """Accept only a well-formed AND/OR expression of reviewed identifiers.

    Cargo's legacy slash syntax is OR. WITH is intentionally unsupported, as
    are LicenseRef, arbitrary prose and copyleft alternatives to permissive terms.
    """
    if not isinstance(expression, str) or not expression.strip():
        return False
    text = expression.replace("/", " OR ")
    tokens = re.findall(r"[A-Za-z0-9.+-]+|[()]|\S", text)
    operand = True
    depth = 0
    for token in tokens:
        if operand:
            if token == "(":
                depth += 1
            elif token in PERMISSIVE:
                operand = False
            else:
                return False
        elif token == ")" and depth:
            depth -= 1
        elif token in ("AND", "OR"):
            operand = True
        else:
            return False
    return bool(tokens) and not operand and depth == 0


def metadata(target=None):
    command = [
        "cargo", "metadata", "--offline", "--locked", "--all-features",
        "--format-version", "1",
    ]
    if target:
        command += ["--filter-platform", target]
    # No shell and no build scripts; cargo metadata resolves manifests only.
    result = subprocess.run(command, check=True, capture_output=True, encoding="utf-8")
    return json.loads(result.stdout)


def roots(data):
    packages = {p["id"]: p for p in data["packages"]}
    members = [packages[i] for i in data["workspace_members"]]
    by_name = {p["name"]: p for p in members}
    for name in sorted(REQUIRED):
        require(name in by_name, f"mandatory permissive root missing: {name}")
        require(by_name[name]["license"] == "MIT OR Apache-2.0",
                f"mandatory permissive root relicensed: {name}")
    selected = []
    for package in members:
        # Discover new permissive workspace members too. Unknown/missing
        # workspace licenses cannot silently fall out of the protected set.
        if package["license"] != "GPL-3.0-or-later":
            require(permissive(package["license"]),
                    f"unreviewed workspace license: {package['name']}: {package['license']!r}")
            selected.append(package["id"])
    return sorted(selected, key=lambda i: packages[i]["name"])


def check_closures(data, target):
    packages = {p["id"]: p for p in data["packages"]}
    nodes = {n["id"]: n for n in data["resolve"]["nodes"]}
    failures = []
    for root in roots(data):
        pending = [(root, [packages[root]["name"]])]
        seen = set()
        while pending:
            package_id, path = pending.pop()
            if package_id in seen:
                continue
            seen.add(package_id)
            package = packages[package_id]
            if not permissive(package["license"]):
                failures.append(
                    f"{target}: {' -> '.join(path)}: "
                    f"{package['name']} {package['version']} license={package['license']!r}"
                )
            for dep in nodes[package_id]["deps"]:
                # Cargo's normal edge kind is JSON null; do not interpret an
                # absent/malformed field as a valid dependency kind.
                require(dep["dep_kinds"], f"missing dependency kinds: {dep}")
                if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                    pending.append((dep["pkg"], path + [packages[dep["pkg"]]["name"]]))
        print(f"{target}: {packages[root]['name']}: {len(seen)} normal-closure packages")
    require(not failures, "non-permissive or missing license metadata:\n" + "\n".join(failures))


def inventory(data):
    """Exact lockfile identities + manifest license declarations, all platforms.

    Cargo.lock has no license fields. Equality with its package identities keeps
    dormant, target-specific, build and dev packages from disappearing unnoticed.
    Non-registry/path dependencies are not exempt from this inventory.
    """
    lock = tomllib.loads(Path("Cargo.lock").read_text(encoding="utf-8"))
    identity = lambda p: (p["name"], p["version"], p.get("source"))
    locked = {identity(p) for p in lock["package"]}
    resolved = {identity(p) for p in data["packages"]}
    require(locked == resolved,
            f"lock/metadata mismatch: lock-only={locked - resolved}, metadata-only={resolved - locked}")
    workspace = set(data["workspace_members"])
    packages = sorted((p for p in data["packages"] if p["id"] not in workspace),
                      key=lambda p: (p["name"], p["version"], p.get("source") or ""))
    lines = [BEGIN, "| Package | Version | Declared license | Source |",
             "|---|---|---|---|"]
    for package in packages:
        license_text = package["license"]
        require(isinstance(license_text, str) and license_text.strip(),
                f"missing license metadata: {package['name']} {package['version']}; review license-file manually")
        source = package.get("source") or "workspace-external path"
        if source == "registry+https://github.com/rust-lang/crates.io-index":
            source = "crates.io"
        cells = (package["name"], package["version"], license_text, source)
        require(all(not re.search(r"[|\r\n`]", cell) for cell in cells),
                f"unrepresentable inventory row: {package['name']}")
        lines.append("| " + " | ".join(f"`{cell}`" for cell in cells) + " |")
    return "\n".join(lines + [END]), len(packages)


def check_notice(expected):
    notice = Path("THIRD_PARTY.md").read_text(encoding="utf-8")
    require(notice.count(BEGIN) == notice.count(END) == 1,
            "THIRD_PARTY.md needs exactly one generated inventory block")
    actual = notice[notice.index(BEGIN):notice.index(END) + len(END)]
    require(actual == expected,
            "THIRD_PARTY.md inventory is missing/stale/wrong; regenerate with "
            "python .github/scripts/license-audit.py --inventory, then review the diff")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", action="store_true",
                        help="print the lock inventory for THIRD_PARTY.md; NOT a license approval")
    args = parser.parse_args()
    require(Path("Cargo.lock").is_file(), "run in checkout root with a committed Cargo.lock")
    for variable in ("RUSTAMP_LICENSE_CRATES", "RUSTAMP_LICENSE_TARGETS"):
        require(variable not in os.environ, f"coverage override is forbidden: {variable}")
    data = metadata()
    block, count = inventory(data)
    if args.inventory:
        print(block)
        return
    check_notice(block)
    print(f"THIRD_PARTY.md: {count} external package versions match Cargo.lock and metadata")
    for target in TARGETS:
        check_closures(metadata(target), target)
    print("OK: all permissive workspace roots, all features, both Windows targets; notices current")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, subprocess.CalledProcessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        sys.exit(1)
