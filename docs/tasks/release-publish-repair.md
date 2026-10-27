# Release publication repair

The CircleCI GitLab release POST contained literal backslash-n characters
instead of shell line continuations. Replace them with actual continuations;
validate the extracted shell syntax without publishing or contacting GitLab.
Also align package source and upgrade links with the configured GitLab remote.
The release job rejects tags that disagree with the workspace version and
copies only the archive and checksum generated for that checkout.
No product or protocol contract changes. Sources: this repository only.

Checks: both shell fragments passed `bash -n`; the tag check accepted the
workspace's `v0.1.0` and rejected `v1.0.0`. Publication was not attempted.
