#!/usr/bin/env python3
"""Metadata-only fixtures. No workspace artifacts, Cargo edits or downloads."""

from contextlib import redirect_stdout
from copy import deepcopy
import io
import os
from pathlib import Path
import runpy
import subprocess
import sys
from unittest.mock import patch


sys.dont_write_bytecode = True
audit = runpy.run_path(str(Path(__file__).with_name("license-audit.py")))
checks = 0


def expect(condition, label):
    global checks
    if not condition:
        raise ValueError(f"fixture failed: {label}")
    checks += 1


def rejects(function, *args, contains=None):
    try:
        with redirect_stdout(io.StringIO()):
            function(*args)
    except ValueError as error:
        expect(contains is None or contains in str(error), f"failure diagnostic: {error}")
    else:
        raise ValueError("fixture unexpectedly passed")


for expression in (
    "MIT", "MIT OR Apache-2.0", "MIT/Apache-2.0",
    "(Apache-2.0 OR MIT) AND BSD-3-Clause", "(MIT OR Apache-2.0) AND Unicode-3.0",
):
    expect(audit["permissive"](expression), f"accept {expression}")

for expression in (
    None, "", " ", "?", "NOASSERTION", "Unknown", "LicenseRef-custom",
    "GPL-3.0-or-later", "AGPL-3.0-only", "LGPL-2.1-or-later", "GFDL-1.3-only",
    "MPL-2.0", "EPL-2.0", "CDDL-1.0", "BUSL-1.1", "MIT OR GPL-3.0-only",
    "MIT OR", "MIT Apache-2.0", "(MIT", "MIT)", "()", "MIT || Apache-2.0",
    "MIT WITH made-up-exception", "mit", "MIT OR ()",
):
    expect(not audit["permissive"](expression), f"reject {expression!r}")

# Pin the actual subprocess flags, including both platform values. No cargo
# subprocess is launched by these mocks.
for target in (None, *audit["TARGETS"]):
    result = subprocess.CompletedProcess([], 0, stdout="{}")
    with patch("subprocess.run", return_value=result) as mocked:
        expect(audit["metadata"](target) == {}, "metadata JSON parsed")
        command = mocked.call_args.args[0]
        expect(all(flag in command for flag in ("--offline", "--locked", "--all-features")),
               "offline/locked/all-features cannot regress")
        expect(target is None or command[-2:] == ["--filter-platform", target], "target flag")

# Resolve real cached manifests once; mutate only in memory, never Cargo.toml.
data = audit["metadata"](audit["TARGETS"][0])
root = next(p for p in data["packages"] if p["name"] == "rustamp-plughost")
for license_text in (None, "", "?", "MPL-2.0", "MIT OR GPL-3.0-only"):
    fixture = deepcopy(data)
    fixture["packages"].append({
        "id": "fixture", "name": "fixture", "version": "1.0.0", "license": license_text,
    })
    fixture["resolve"]["nodes"].append({"id": "fixture", "deps": []})
    next(node for node in fixture["resolve"]["nodes"] if node["id"] == root["id"])["deps"].append({
        "pkg": "fixture", "dep_kinds": [{"kind": None, "target": None}],
    })
    rejects(audit["check_closures"], fixture, audit["TARGETS"][0],
            contains="rustamp-plughost -> fixture")

fixture = deepcopy(data)
next(p for p in fixture["packages"] if p["id"] == root["id"])["license"] = "GPL-3.0-or-later"
rejects(audit["roots"], fixture, contains="relicensed")
fixture = deepcopy(data)
fixture["workspace_members"].remove(root["id"])
rejects(audit["roots"], fixture, contains="root missing")
fixture = deepcopy(data)
fixture["packages"].append({"id": "new-root", "name": "new-root", "license": "MIT"})
fixture["workspace_members"].append("new-root")
expect("new-root" in audit["roots"](fixture), "additional permissive root discovered")
fixture["packages"][-1]["license"] = None
rejects(audit["roots"], fixture, contains="unreviewed workspace license")

expected = audit["BEGIN"] + "\nexpected\n" + audit["END"]
with patch.object(Path, "read_text", return_value=expected):
    audit["check_notice"](expected)
    expect(True, "current notice passes")
for notice in ("", expected + expected, expected.replace("expected", "stale")):
    with patch.object(Path, "read_text", return_value=notice):
        rejects(audit["check_notice"], expected)

with patch("subprocess.run", side_effect=subprocess.CalledProcessError(101, "cargo")):
    try:
        audit["metadata"]()
    except subprocess.CalledProcessError:
        expect(True, "cargo resolution failure propagates")
    else:
        raise ValueError("cargo error was swallowed")

with patch.object(Path, "is_file", return_value=False), patch("sys.argv", ["license-audit.py"]):
    with patch("subprocess.run") as mocked:
        rejects(audit["main"], contains="Cargo.lock")
        expect(not mocked.called, "missing lock fails before Cargo")
for variable in ("RUSTAMP_LICENSE_CRATES", "RUSTAMP_LICENSE_TARGETS"):
    with patch.dict(os.environ, {variable: " "}), patch("sys.argv", ["license-audit.py"]):
        rejects(audit["main"], contains="coverage override")

print(f"OK: {checks} metadata-only regression checks; no workspace executable or test suite run")
