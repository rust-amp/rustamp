# Playlist resize repair

Goal: keep the playlist skin, controls and viewport aligned with its client
size while dragging and after restoring saved dimensions. The default-sized
frame remains unchanged. Own paths: playlist UI and app presentation. Sources:
audited S01, S05, S06 and S12; no dirty sources or copied third-party code.
Validation: headless rendering, input, scroll and resize regressions; GUI
verification in the interactive VM. No shared core or protocol contract change.

Owner override (2026-10-27): when global 2x is enabled, scale the playlist's
skin and coordinates too if both dimensions are smaller than 550x232. Larger
playlists retain their size. This is an explicit product choice that differs
from S05's measured reference behavior. Turning 2x off restores the prior size
unless the playlist was manually resized while scaled.

Headless verification: 2,336 workspace library tests passed (excluding the
device-probing output crate), as did both 18-case built-in golden suites and
17 base-2.91 acceptance cases. Format and workspace Clippy passed.
Interactive VM verification: `--ignored:rustamp_app` passed all seven tests in
session 1, including the extended resize and 2x window test. The proof is at
`D:\code\rustamp-vm\staging\out\resize-agent4\gui-session\session-proof.json`.
The scheduled broker is disabled; the owner authorized direct execution of the
same allow-listed broker under the user's Hyper-V permissions. The full suite
remains a separate check.
