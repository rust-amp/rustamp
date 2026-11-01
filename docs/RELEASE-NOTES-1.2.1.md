# Rustamp 1.2.1

The player is the same as in [1.2.0](RELEASE-NOTES-1.2.0.md), apart from the two
fixes below.

- The project moved to GitHub: https://github.com/rust-amp/rustamp. The links in
  the installer, the bundled documents and the source offer point there.
- Releases are built by GitHub Actions.
- The last frames of a stream are no longer lost when the sending side publishes
  them and closes the ring at the moment the reader finds it empty. This was
  rare: it showed in about 1 of 5760 runs of a stress test under load.
- A control client that reads the snapshot right after a playlist event now sees
  the change; it could read the state from before it.

The list of what works, and of what 1.2.0 does not do, is in the
[1.2.0 notes](RELEASE-NOTES-1.2.0.md) and holds for this release.
