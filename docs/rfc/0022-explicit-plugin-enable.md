# RFC 0022: plug-ins are switched on explicitly

Status: **Proposed** (2026-10-31). Acceptance belongs to the integration lead.

## Goal

Approving a plug-in records that its exact image may run. It must not also make
it run. With the off-list `PluginsConfig::disabled`, every approved plug-in that
was not on the list was active, so approving an output plug-in replaced the
audio output without being asked.

## Decision

- `rustamp_core::config::PluginsConfig::disabled` is replaced by
  `enabled: Vec<String>`, the file names the user switched on. A plug-in runs
  when its image is approved **and** its file is on the list.
- `CONFIG_VERSION` becomes 2.
- At most one output plug-in is on the list. No output on the list means the
  player's own output. There is no separate key for the output.

## Migration

A file with version 1 or no version is migrated when it is loaded:

- `enabled` becomes the names in the approval store minus the old off-list, so
  what ran before the upgrade keeps running.
- Of several outputs, the first by file name stays on, which is the one that
  played under the old rule.
- A store that cannot be read approves nothing: nothing is carried over as on,
  and the load warns.
- The next save writes version 2 and drops `plugins.disabled`.

A file of version 2 or newer is not migrated. A list that names several outputs
by hand keeps the first.

## Consumers

`rustamp-prefs` (`is_enabled`, `set_enabled`) and `rustamp-app` (the
configuration file, the Preferences pages) change with this RFC. No other crate
reads the field.
