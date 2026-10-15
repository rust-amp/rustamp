#!/usr/bin/env bash
#
# RFC 0004 acceptance 4: no crate in the permissive set may depend, directly
# or transitively, on a GPL-licensed crate.
#
# The permissive set is rustamp-proto, rustamp-plugin-abi, rustamp-ipc and
# rustamp-plughost (RFC 0004 section 1). They are the crates a plugin author or
# a redistributor of the 32-bit plugin host receives, so a copyleft crate
# anywhere in their dependency closure would put the licensing split at risk.
# The check proves the invariant from the dependency graph, not by inspection.
#
# It needs nothing but cargo: the license strings are read out of the resolved
# graph with `cargo tree`. No extra tool, no extra dependency, and no network
# beyond what `cargo tree` itself needs (none while Cargo.lock is current).
#
# Usage, from the repository root (CI calls exactly this):
#
#     bash .github/scripts/check-permissive-licenses.sh
#
# On Windows, run it from **Git Bash**. The `bash` in `System32` is WSL, whose
# PATH cannot see the Windows `cargo`, so the script would report every closure
# as unresolvable there.
#
# Exit status 0 means every closure of every crate on every target is clean.
# Anything else exits 1 and names the offending package. Two conditions fail:
# a copyleft license string, and a package whose license metadata is missing or
# unparsable, because an unknown license cannot be shown to be permissive. The
# check fails closed; it never promotes a gap to a fact.
#
# The closure cargo prints includes the root crate itself, so the check also
# catches one of the four crates being relicensed by accident.
#
# `-e normal` with the default features is the *complete* declared closure of
# these four crates today: none of them has an optional dependency or a feature
# that gates one (checked 2026-10-15). If a feature-gated dependency is ever
# added to one of them, this check must gain `--all-features` to stay complete.
#
# The crate list and the target list can be overridden, which is how the
# script's own failure path is exercised:
#
#     RUSTAMP_LICENSE_CRATES="my-crate" \
#     RUSTAMP_LICENSE_TARGETS="x86_64-pc-windows-msvc" \
#     bash .github/scripts/check-permissive-licenses.sh

set -euo pipefail

# The four MIT OR Apache-2.0 crates (RFC 0004 section 1).
read -r -a crates <<< "${RUSTAMP_LICENSE_CRATES:-rustamp-proto rustamp-plugin-abi rustamp-ipc rustamp-plughost}"

# The targets the permissive crates must build for (RFC 0004 section 3). The
# closure is resolved per target, because a target-specific dependency only
# appears under its own target (rustamp-ipc's `windows` dependency, for one).
read -r -a targets <<< "${RUSTAMP_LICENSE_TARGETS:-x86_64-pc-windows-msvc i686-pc-windows-msvc}"

# Copyleft markers, matched case-insensitively against the license expression.
# `A?GPL` covers GPL, AGPL and LGPL (the substring); `GFDL` is spelled out
# because it contains neither. Failing on LGPL as well is deliberate: this is a
# fail-closed check, not a weighing of weak copyleft.
is_copyleft() {
    printf '%s' "$1" | grep -Eiq '(A?GPL|GFDL)'
}

status=0

for crate in "${crates[@]}"; do
    for target in "${targets[@]}"; do
        printf '== %s on %s\n' "$crate" "$target"

        # `-e normal` restricts the closure to the crates linked into the
        # artifact; build- and dev-dependencies are out of scope here (task
        # card V1). `{p}` is the package (name, version, source), `{l}` its
        # license expression as declared in its manifest.
        if ! tree="$(cargo tree -p "$crate" --target "$target" -e normal --format '{p} || {l}')"; then
            printf 'FAIL: cargo tree could not resolve the closure of %s on %s\n' \
                "$crate" "$target" >&2
            exit 1
        fi

        packages=0
        closure_status=0
        while IFS= read -r line; do
            [ -n "$line" ] || continue
            packages=$((packages + 1))

            if [[ "$line" != *' || '* ]]; then
                printf 'FAIL: %s on %s: unparsable cargo tree line (no " || " separator): %s\n' \
                    "$crate" "$target" "$line" >&2
                status=1
                closure_status=1
                continue
            fi

            # Strip cargo's tree drawing prefix ("  |   +-- "), which is not
            # part of the package name. Cargo package names always start with
            # an alphanumeric character, so dropping everything before the
            # first one is safe in any locale.
            package="$(printf '%s' "${line%% || *}" | sed -E 's/^[^[:alnum:]_]+//')"
            licence="${line##* || }"
            # cargo appends " (*)" to a package it already expanded further up
            # the tree. That is tree drawing, not part of the license expression.
            licence="${licence% (\*)}"

            if [ -z "$licence" ]; then
                printf 'FAIL: %s on %s: %s declares no license metadata; an unknown license is not a permissive one\n' \
                    "$crate" "$target" "$package" >&2
                status=1
                closure_status=1
                continue
            fi

            if is_copyleft "$licence"; then
                printf 'FAIL: %s on %s: %s is licensed "%s"; RFC 0004 acceptance 4 forbids a copyleft crate in this closure\n' \
                    "$crate" "$target" "$package" "$licence" >&2
                # Best effort: show why it is in the closure. Failures of this
                # diagnostic do not change the verdict.
                cargo tree -p "$crate" --target "$target" -e normal -i "${package%% *}" || true
                status=1
                closure_status=1
            fi
        done <<< "$tree"

        if [ "$closure_status" -eq 0 ]; then
            printf '   %s packages: no copyleft, no missing license metadata\n' "$packages"
        else
            printf '   %s packages: violations above\n' "$packages"
        fi
    done
done

if [ "$status" -ne 0 ]; then
    printf '\nRFC 0004 acceptance 4 is violated. rustamp-proto, rustamp-plugin-abi, rustamp-ipc and rustamp-plughost are MIT OR Apache-2.0, so no GPL/AGPL/GFDL crate may enter their dependency closure. Replace the offending dependency, or move the code that needs it behind the process boundary.\n' >&2
    exit 1
fi

printf '\nOK: the four permissive crates stay GPL-free on every target above.\n'
