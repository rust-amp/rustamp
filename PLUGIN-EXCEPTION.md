# GPL-3.0 additional permission: the Rustamp plugin exception

This is an **additional permission under section 7 of the GNU General Public
License, version 3** for Rustamp's own GPL-licensed code (RFC 0004, and
`docs/notes/L-licensing-plugins.md` section 4.1). It supplements the GNU GPL
for the covered work; it does not replace it and it does not relax any term for
third-party material.

## The permission

> As an additional permission under section 7 of the GNU General Public License,
> version 3 (or, at your option, any later version), the copyright holders of
> Rustamp give you permission to combine the covered work with independent
> modules that communicate with the covered work solely through (a) the
> documented Rustamp classic-plugin ABI (the Winamp 2.x `in_`/`out_`/`vis_`/
> `dsp_`/`gen_` plugin interface, as defined by the `rustamp-plugin-abi` crate)
> and (b) the documented Rustamp IPC protocol (the `rustamp-ipc` crate,
> protocol version 1), regardless of the license terms of those independent
> modules, and to copy and distribute the resulting combination under the terms
> of the GNU GPL for the covered work and the licenses of the other code
> concerned, provided that you do not modify those documented interfaces. This
> exception does not apply to material in the covered work that is licensed
> under the GPL by third parties; as to that material, the GPL applies without
> this exception.

## Scope

The permission applies to the GPL-3.0-or-later crates of Rustamp that contain
**only Rustamp's own code**:

`rustamp-core`, `rustamp-audio`, `rustamp-dsp`, `rustamp-output`,
`rustamp-meta`, `rustamp-playlist`, `rustamp-gfx`, `rustamp-ui`, `rustamp-vis`,
`rustamp-milkdrop`, `rustamp-platform`, `rustamp-prefs`, `rustamp-library`,
`rustamp-app`, `rustamp-testkit`.

It is deliberately **not** attached to `rustamp-decoders` (which ports
third-party decoder sources) or to `rustamp-skin` (which carries third-party
webamp MIT code, so it is not "only our own code"): only a copyright holder may
grant this exception for their own code, so a crate that includes someone else's
code cannot extend the permission to that code.

The plugin-facing crates `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`
and `rustamp-plughost` do **not** need this permission: they are licensed
`MIT OR Apache-2.0` and carry no copyleft at all (RFC 0004). The permission
exists for the GPL player, so that the player ↔ plugin-host edge is covered for
Rustamp's own code even under the strictest reading of the FSF's plugin and
shared-memory positions.

## Notes

* The permission is an *additional permission*: GPL-3.0 section 7 lets anyone
  who conveys a copy remove it, and modified versions need not carry it. The
  permissive license on the plugin-facing crates is the durable instrument; this
  permission is the companion for the player.
* Because the project is GPL-3.0-**or-later**, the permission is worded to apply
  under any version of the GPL the recipient elects.
* This document is not legal advice. The FSF's own publications say the same of
  theirs. See `docs/notes/L-licensing-plugins.md` for the reasoning and the
  sources.
