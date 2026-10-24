---
Spec ID: S-13
Title: Command-line switches, single instance, drag-and-drop rules
Sources consulted (category only): dirty room: disassembly of the reference binary (owner-authorised under RFC 0021); dirty room: string table of the reference binary
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Second audit pass: 2026-10-20 — adds the `/ADD` switch, corrects the absence claim that followed from the first pass, and scopes the completeness of the switch list.
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S13 — command-line switches, single instance, drag-and-drop rules

## 0. How to read this spec

**The reference binary was not run for this spec.** Everything below is read out
of the reference's own strings and out of its own switch table. No behaviour was
observed at runtime, so nothing here is a measurement. Where a statement goes
beyond what a string or a table entry can carry on its own, it is marked
**reading** — a hypothesis, never a fact to implement.

| Where it comes from | What it means |
|---|---|
| §1, §3 | **String fact** — a spelling the reference carries: a switch name, a ProgID, a verb name, a registry key shape, a registered command line. |
| §1, §2 | **Table fact** — what the reference's own switch table pairs with a switch. |
| a statement marked **reading** | A reading of a name, or an inference drawn from the table. Not observed behaviour. |
| §5 | **Open** — not established here; the measurement that would settle it is named. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing.

## 1. The command line

The reference recognises the switches below. Anything else on the command line
is treated as a file argument.

| Switch | Form | What the reference pairs it with |
|---|---|---|
| `/CLASS=` | `/CLASS=<name>` | the window class the single-instance check looks for (§2) |
| `/CONFIG=` | `/CONFIG=<path>` | the configuration file to use instead of the default |
| `/BOOKMARK` | `/BOOKMARK <path>` | adds the path as a bookmark rather than playing it |
| `/ADD` | `/ADD <path>` | the `Enqueue` verb of §3: the reference registers that verb with the command line `"%s" /ADD "%%1"` — the program path, the switch, then the item's path. What the switch *does* is a **reading** of the name — "enqueue, do not play", the counterpart of `/BOOKMARK` |
| `/INSTALL` | flag | the installer association step — the shell registration of §3 |
| `/UNREG` | flag | removal of the shell registration |
| `/NOREG` | flag | skipping the registration step |
| `/ONLYINSTALL` | flag | the install step, then exit without starting playback |

Facts about the forms:

- A switch is a leading `/`.
- The two switches that take their value after an `=` spell that value
  directly after the `=`, with no space. The value runs to the end of the
  argument and may contain spaces (`/CLASS=<name>`).
- `/ADD` and `/BOOKMARK` take their path as the argument that follows the
  switch, not after an `=`.
- The bookmark form is registered as `"%s" /BOOKMARK "%%1"` — the program path,
  the switch, then the item's path — and the stored template is
  `/BOOKMARK %s`, in which `%s` stands for the path. The two writings do not
  agree on their quoting — see §5.
- The `Enqueue` verb of §3 is registered with the same shape,
  `"%s" /ADD "%%1"`.
- A command line may carry several paths; the switches and the paths are
  mixed.
- **No** `/PLAY`, `/NEW`, `/EJECT` or `/TASKBAR` is in the reference's switch
  list. A command line that carries one of them is not interpreted as a
  switch. The corresponding behaviours are reached through the shell verbs of
  §3 rather than through the command line; which verb stands in for which
  missing switch is not established. `Enqueue` is the exception: it is not a
  behaviour that only a verb reaches, it **is** the `/ADD` switch of this
  table.
- **Scope of the list.** The list above is the reference's own switch list.
  It is not proven to be every switch the reference understands: `/ADD` was
  found only because the reference spells it inside the `Enqueue` verb's
  registered command line (§3), and a switch the reference spells nowhere
  else would not have been found at all. See §5.

What a file argument does is S02's: that spec measured that a file given on the
command line starts playing it, and that an entry passed while the player is
stopped is loaded and shown in the marquee (S02 §1). What several file
arguments do together is not covered by that measurement.

## 2. Single instance

- The player registers a **window class**, and the class name is what
  `/CLASS=` overrides. A second start looks for a window of that class and hands
  its arguments to the running instance instead of opening a second player.
- The default class name is the main window's class: `Winamp v1.x`. **S09 is
  the normative home for that name** — S09 §1 (the parent-window member) and
  S09 §8 item 1 (the window classes) give it as an ABI fact that plugins and
  external tools depend on, and S01 §1 lists it in the window-geometry table.
  It is written down here once, with the pointer, so that the single-instance
  check and the plugin-facing window cannot drift apart.
- The class name is a case-sensitive string.
- The hand-over is a window message. **Which message it is, is not stated
  here**: S10 covers the plugin-facing IPC channel (`WM_WA_IPC`, `WM_COPYDATA`,
  `WM_COMMAND`) and not the single-instance hand-over, so no audited spec
  currently names it. See §5.

## 3. Shell registration — what makes a drop from Explorer work

The reference registers shell verbs per file type and per directory. The handler
ProgIDs it names are `Winamp.File`, `Winamp.PlayList` and `Winamp.SkinZip`; the
directory case appears under its own identifiers.

| Handler | ProgID | Verbs |
|---|---|---|
| media file | `Winamp.File` | `open`, `Play`, `Enqueue` (→ `/ADD`), `ListBookmark` |
| playlist file | `Winamp.PlayList` | `open`, `Play`, `Enqueue` (→ `/ADD`), `ListBookmark` |
| skin archive | `Winamp.SkinZip` | `install` |
| directory | the directory handler's own identifiers | `Play`, `Enqueue` (→ `/ADD`) — also named `Winamp.Play` / `Winamp.Enqueue` |

So a media file and a playlist file each get **four** verbs, a skin archive
gets **one**, and a directory gets **two**.

Facts that constrain a compatible implementation:

- `Play` and `Enqueue` are **separate verbs**, so the user chooses between
  them. What they do is a **reading** of the names: play replaces the playlist
  and starts, enqueue appends to the playlist. For `Enqueue` the reading is
  corroborated by the switch the verb is registered with (`/ADD`, §1); for
  `Play` it stands on the name alone.
- The `Enqueue` verb is not a mechanism next to `/ADD` — it **is** `/ADD`.
  The reference registers the verb with the command line `"%s" /ADD "%%1"`:
  the program path, the switch, then the item's path. The arrow in the table
  marks this. The verb that registers `/BOOKMARK` has the same shape,
  `"%s" /BOOKMARK "%%1"`. The registered command lines of the other verbs are
  not recorded (§5).
- Both verb keys carry a `DropTarget` value — `…\shell\Play\DropTarget` and
  `…\shell\Enqueue\DropTarget` — so the player is the drop target for both
  verbs, and a drop on the player's shortcut offers two possible meanings.
  What a drop on the player's own window does is a different question and is
  open (§5). What the `DropTarget` entries carry as their value is not
  recorded (§5).
- The label `Enqueue` is the display name of that verb. The display names of
  the other verbs are not recorded (§5).
- A playlist file has the same four verbs as a media file, so a dropped `.m3u`
  is offered the same choice as a dropped `.mp3`.
- Beyond `open`, `Play` and `Enqueue` there is a fourth verb, `ListBookmark`:
  it lists the item's bookmark rather than playing it (**reading** of the
  name).
- `SkinZip` has `install`, which is how a dropped `.wsz` becomes a skin
  (**reading** of the name).
- **Reading:** registration writes the same key set each time, so registering
  again changes nothing, and `/UNREG` is the matching removal step for those
  keys. Neither was observed in a running shell.

## 4. Practical notes for an implementer

- Parse the command line as: a leading `/` starts a switch, `=` separates a
  switch's value, a switch may take the argument that follows it as its path
  (`/ADD <path>`, `/BOOKMARK <path>`), everything else is a file argument
  (§1).
- Before starting a player, look for a window of the class of §2 and hand the
  arguments over instead of opening a second one (§2).
- Do not implement the switches §1 lists as absent, and do not add others:
  the list is the reference's own switch list, and whether it is every switch
  the reference understands is open (§5).
- Register the verbs of §3 under the ProgIDs of §3, with a `DropTarget` value
  under the `Play` and the `Enqueue` key.
- Everything in §5 is unknown. A behaviour that §5 lists must not be
  implemented as if it were settled.

## 5. Open questions

1. **The in-window drop rule.** Dropping files onto the main window, the
   playlist and the equalizer may differ (replace vs append, or a prompt). The
   verb table of §3 says only what Explorer offers; it does not say what the
   window does with `WM_DROPFILES`. **A VM observation is needed** and is the
   next measurement.
2. **The drop of a folder**: play recursively, or add the folder?
3. **`/BOOKMARK` without a path.**
4. **Whether `/CONFIG=` also moves the skin/plug-in directories** or only the
   ini file.
5. **What the second instance does when the first is busy** — still hands over,
   or shows a dialog.
6. **Whether `/CONFIG=` affects the class** that the single-instance check
   looks for.
7. **The default value of `/CLASS=`.** The default class name is published as
   an ABI fact in S09 §1 and §8 item 1 (§2), but what the reference's own
   switch table carries as the default was not read out, so the name in §2
   comes from S09 and not from the switch table.
8. **Which window message carries the hand-over** (§2). S10 covers the
   plugin-facing IPC channel, not this.
9. **The exact quoting of the bookmark command line** (§1): the registered form
   is `"%s" /BOOKMARK "%%1"`, the template with no quotes at all.
10. **What `Winamp.Play` and `Winamp.Enqueue` are** (§3, the directory row).
11. **The display names of the remaining verbs, and the registered command
    lines of the verbs other than `Enqueue`** (§3).
12. **What the `DropTarget` entries carry** (§3): only the key shape is
    recorded.
13. **Whether the switch list of §1 is complete.** The list is what the
    reference's switch names yield; `/ADD` was found only because the
    reference spells it inside the `Enqueue` verb's registered command line.
    Whether the reference carries a switch that is spelled nowhere else is
    not established.
14. **The name of the verb that registers `/BOOKMARK`.** The draft calls it
    the `Bookmark` verb, §3's table lists `ListBookmark`. Whether these are
    one verb or two is not established — and it matters, because a second verb
    would change §3's counts.
