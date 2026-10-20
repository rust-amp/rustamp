---
Spec ID: S-16
Title: Preferences pages and options
Sources consulted (category only): dirty room: the dialog resource strings of the reference binary (UTF-16 string table); dirty room: a configuration file written by the reference
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S16 — the Preferences pages and their options

## 0. How to read this spec

**The reference binary was not run for this spec, and no window was opened.**
Everything below is read out of the reference's own **dialog resource strings**
— the page labels and the per-option help texts — and cross-checked against
the **configuration key names** the player recognises (S12). No behaviour was
observed at runtime, so nothing here is a measurement of a running player.
Where a statement goes beyond what a string or a key name can carry on its
own, it is marked **reading** — a hypothesis, never a fact to implement.

| Where it comes from | What it means |
|---|---|
| §1–§3 | **String fact** — a spelling the reference carries: a page label, an option's help sentence, a dialog prompt, a button label. |
| §4 | **Key fact** — a configuration key name the player recognises (S12 §5.1), and what that name matches. |
| a statement marked **reading** | A reading of a name or of a help text — what a string implies, not what was observed. |
| §5 | **Practical note** — a consequence that follows from the spec's own facts; nothing new. |
| §6 | **Open** — not established here; the measurement that would settle it is named. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing. What the audit
changed, and what it found, is in `docs/notes/spec-audit-s16.md`.

**Scope.** This spec is about the **option inventory** of the reference's
Preferences: which pages the resource table names, which options each of them
carries, and the help text each option shows. **Layout and control types are
out of scope** — the dialog's control types (checkbox, radio, edit, dropdown),
their order, their placement and the dialog's geometry were not read; that
needs the window (§6, open question 2).

**Our Preferences is our own design.** The reference's page set is fixed and
its layout is not reproduced here. What this spec fixes is the **inventory**,
so that an option the reference offers and we do not is visible as a gap
(§5) — not the page tree, the wording of our pages, or the look of the dialog.

## 1. The page labels

The reference's resource table carries these **ten** labels, in this order:
`Setup`, `File types`, `Options`, `Display`, `Plug-ins`, `Input`, `Output`,
`Visualization`, `DSP/Effect`, `General Purpose`.

Further in the same table sit the strings of the **skin-related page**
(§3.3) and of a **video page** (§3.8), of an **agent-and-network group**
(§3.4), of an **Internet-connection group** (§3.5) and of a **language-pack**
item (§3.7). Those five are **not** among the ten labels above.

The library is a **plug-in with a window of its own**, not a page of the
Preferences dialog (S20 §0, §1); no library page is in this table.

### 1.1 What the order does and does not tell us

The labels are a flat sequence. Which label is a **top-level** page and which
is a **child** is not established by the strings. The most plausible parents
are marked **inferred** below:

- **Setup** — with `File types` as a child;
- **Options** — with `Display` (and the skin-related page of §3.3) as
  children;
- **Plug-ins** — with `Input`, `Output`, `Visualization`, `DSP/Effect`,
  `General Purpose` as children.

**Reading, not a fact.** The exact tree is **open** (§6, open question 1).

### 1.2 The labels against the sections — five and five

The inventory of §1 and the option lists of §3 do **not** cover each other.
This is the spec's own cross-check, and it is published rather than smoothed
over, because an implementer who trusts the ten labels alone will miss five
option groups, and one who trusts the option lists alone will miss five
labels.

| Label of §1 | Section that carries its options |
|---|---|
| `Setup` | §3.1, §3.4 |
| `File types` | §3.1 |
| `Options` | §3.2, §3.3 |
| `Display` | §3.2 |
| `Plug-ins` | §3.6 |
| `Input` | — none |
| `Output` | — none |
| `Visualization` | — none |
| `DSP/Effect` | — none |
| `General Purpose` | — none |

| Section | Page or group it describes | In the label list of §1? |
|---|---|---|
| §3.1 | `Setup` / `File types` | yes |
| §3.2 | `Options` / `Display` | yes |
| §3.3 | skins (under `Options`) | **no** |
| §3.4 | agent and network (under `Setup`) | **no** |
| §3.5 | Internet connection | **no** |
| §3.6 | `Plug-ins` | yes |
| §3.7 | language pack | **no** |
| §3.8 | Video | **no** |

So **five** of the ten labels have no option list in this spec, and **five**
of the option groups have no label in the ten. Which of the five unlabelled
groups are pages of the Preferences dialog, and which are something else
(a dialog, a list, a control group), is not established here (§6, open
question 8).

### 1.3 What the repository already holds

The tree carries **partial measurements of the reference's Preferences
window** that were taken in other lanes and were **not** part of this spec's
evidence:

- `docs/HANDOFF.md` (VM run `20261016-204109`) records the Preferences family
  as one **443 × 336** frame whose pages are pixel-distinguishable, that bare
  `Ctrl+P` opens the **Setup** page, and that the skin-browser command opens
  the **Skins** page of that same frame rather than a separate window.
- `docs/rfc/0005-app-commands.md` §7 records two page paths measured on
  2026-10-15: command 40191 opens the dialog at **Plug-ins > Visualization**,
  and command 40317 opens the same dialog at **Options > Visualization**.

Neither record is a measurement this spec publishes — the run record behind
the RFC's measurement is not in this repository — and this spec therefore
does **not** settle the tree from them. They are named here because they
already bear on the inference of §1.1: the first path agrees with it, and the
second does not follow from the ten labels at all. The next measurement should
start from these records instead of repeating them.

## 2. How the Preferences window is reached

- The player carries a menu entry **Preferences...** (S04 §4.1, row 1, with
  the hint `Ctrl + P`); the resource string spells it with an accelerator
  marker, `&Preferences...`.
- The shortcut is **`Ctrl+P`**, command id **40012** (S03 §1).
- The last page shown is remembered in the key `prefs_last_page`: one integer
  whose **low byte** is the page index restored at start, and whose bits from
  bit 8 upwards carry a second number (S12 §3.7 and S12 §7). What the second
  number means is not established — **open** (S12 §10, open question 7).
- `Ctrl+K` (40191, "Select visualization plug-in") opens the Preferences
  **dialog** rather than a dialog of its own (S03 §1 — that row is a
  *command* row there, so its effect was exercised by posting the command
  id).

## 3. The options, by their own help texts

Every item below is a **user-facing string** the reference carries. The
help sentence names the option's meaning more precisely than any label
could; the labels themselves are short and are not in the resource table.

### 3.1 Setup / File types

| Option | Help text |
|---|---|
| associate audio CDs | "This option will associate Winamp with Audio CDs in explorer, so that when you insert a disc or click on the disc icon in explorer Winamp will automatically play it." |
| retake file types | "This option will make Winamp retake its filetypes when other programs steal them. It is recommended." |

### 3.2 Options / Display

| Option | Help text |
|---|---|
| skinnable title font | "This option makes Winamp use it's own (skinnable) font for displaying track titles. Disable this if you want to be able to see titles with non-english characters in them." |
| snap distance | "This value controls at what distance (in pixels) Winamp windows snap to each other and to the viewport." |
| playlist advance | "This option controls whether Winamp automatically advances to the next track or not. You only want to enable it if you are DJing and want to be able to jump around without worrying about the playlist auto-advancing." |
| metadata read timing | "Reads track info such as title or ID3 tag, as well as a track length estimate, on play only (rather than on view or load)." |
| shuffle morph rate | "This controls how quickly the playlist order changes (morphs) over time, when shuffle is on." |

**The shuffle morph rate is a reading, not a fact about the algorithm.** The
help text says the playlist order *changes (morphs) over time* while shuffle
is on and that this setting controls how quickly. That is what the **text**
says; it is not a measurement of how the reference shuffles. S15 §1 describes
shuffle as a **permutation without replacement**, generated when playback
starts or when shuffle is toggled on — a model in which the order does not
drift. A morphing order would be a different traversal model, so the two
cannot both be right and nothing here settles which is.

What **S15 would need**: a runtime measurement of the reference — play a
playlist with shuffle on, record the playback order over time at two
different morph-rate settings, and see whether the order drifts or stays a
fixed permutation — plus the setting's range, unit and default, which S12
§10 item 3 leaves open for every numeric key. Until that measurement
exists, this spec records the **text** and not the behaviour (§6, open
question 11). The configuration key is `shuffle_morph_rate` (S12 §5.1).

### 3.3 Options / skins

| Option | Help text |
|---|---|
| skin list | "This box lists available skins to enable." |
| random skin | "When this is checked, Winamp will switch skins randomly on every new file played." |
| skin directory | "This button lets you change the directory where you keep your skins." |

The page has no label in the ten of §1; §1.3 records that the repository's
own run log names it the **Skins** page.

### 3.4 Setup / agent and network

| Option | Help text |
|---|---|
| usage reporting | "This option, when checks, allows Winamp to report anonymous usage information back to Nullsoft. It simply reports how much Winamp is being used, but not how you are using it." |
| agent | "This option enables the Winamp Agent, which runs at all times and lets you have enhanced functionality." |
| tray icon | "This option, when the Agent is enabled, shows an icon in the system tray that lets you run Winamp, browse your bookmarks, etc." |
| agent keeps file types | "This option lets the Agent maintain Winamp's file associations, so that if other software is installed that uses the same extensions, Winamp will keep them." |
| update check | "This option when enabled will have Winamp automatically detect whether a new version has been released and notify you of it." |
| QuickLaunch | "This button will create a Winamp link in your Win98/IE4+ QuickLaunch bar." |

The quoted text of the first row is the reference's own, **including its
grammar**: the wording "when checks" is the reference's own typo, and the
quote is carried verbatim with it. The attribution rests on the draft — the
auditor did not re-read the string out of the binary — so the dirty room
should confirm the exact spelling before an implementer copies the sentence
into our own UI.

### 3.5 Internet connection — three settings

| Setting | Help text |
|---|---|
| LAN | "This setting tells Winamp that Internet access is always available, i.e. if you are connected on a LAN." |
| dial-up | "This setting tells Winamp that Internet access is dependent on having a dial-up connection present, i.e. if you have a modem you use to connect to the Internet." |
| never | "This setting tells Winamp that Internet access is never available." |

**Reading:** the three texts describe one property — whether Internet access
is available — in three states, and the states therefore look mutually
exclusive. The material does not say that explicitly, and the player keeps
**one** key for them, `inet_mode` (S12 §5.1, a key the observed file did not
carry — S12 §5.2); how the three states are encoded in that key is open
(§6, open question 9).

### 3.6 Plug-ins

| Option | Text carried by the resource table |
|---|---|
| visualization directory | "This button lets you select the directory for Winamp visualization plug-ins. The default is `<yourwinampdirectory>\Plugins`" |
| DSP/effect directory | "This button lets you select the directory for Winamp DSP/effect plug-ins. The default is `<yourwinampdirectory>\Plugins`" |
| visualization priority | a priority choice with the values **Idle / Normal / High** |
| flush plug-in name cache | "Flush plug-in name cache" |

The first two rows are help sentences, the third is a **set of choice
values** and the fourth a **button label**; the table's column head says so.
Whether the priority choice is a page-level option or a per-plug-in one is
open (§6, open question 6); the player's recognised key set carries two
candidate names for it, `visplugin_priority` and `priority` (S12 §5.1).

### 3.7 Language pack

| Item | Text |
|---|---|
| language pack | "Open language pack (select Winamp.exe for English)" and "Note that a restart of Winamp is required for the language pack to take effect." |

### 3.8 Video

The video page carries options whose texts the resource table carries in
shortened form:

| Option | Text carried by the resource table |
|---|---|
| sync to the vertical blank line | "This option enables video to sync with the vertical blank line for smoother display." |
| aspect ratio correction | "This option corrects the aspect ratio of the videos so they display at the correct size." |

The rest of the page's texts were truncated by the resource table's own
formatting and are **open** (§6, open question 3). The video page has no
label in the ten of §1.

### 3.9 Dialogs the same table carries (not Preferences pages)

Recorded because they are user-facing text an implementation may need
verbatim:

- playlist removal: "Using this command will remove all selected files
  PERMANENTLY from disk. Are you SURE you want to do this?"
- EQ preset menu: "Load auto-load preset", "Save auto-load preset",
  "Delete auto-load preset", "Delete preset".

## 4. The options and the configuration keys — a cross-check

Checked against S12 §5.1 — the complete set of names the player recognises,
spelled as they appear in the file — the option inventory of §3 and the
recognised key set are **not the same set**, in both directions. The table
below is the check; every entry in its second column is a **reading of a
name**, never a measurement of a setting.

| Option (page) | Key in S12 that matches by name | Status |
|---|---|---|
| playlist advance (§3.2) | `pladv` | match (S12 §3.3) |
| shuffle morph rate (§3.2) | `shuffle_morph_rate` | match (S12 §5.1; recognised but not itemised in S12 §3 — S12 §5.3) |
| random skin (§3.3) | `randskin` | match (S12 §3.1) |
| language pack (§3.7) | `langpack` | match (S12 §5.1) |
| sync to the vertical blank line (§3.8) | `video_vsync` | match (S12 §3.6) |
| aspect ratio correction (§3.8) | `video_aspectadj` | match (S12 §3.6) |
| update check (§3.4) | `newverchk`, `newverchk2` | **two keys for one option** (both inferred — S12 §3.7) |
| LAN / dial-up / never (§3.5) | `inet_mode` | **one key for three settings** |
| skinnable title font (§3.2) | `bifont` | name-based candidate only; the meaning of `bifont` is not established (S12 §5.2 lists it as a key the observed file did not carry) |
| snap distance (§3.2) | `snap`, `snaplen` | name-based candidate only; the meaning of the two keys is not established (S12 §5.2) |
| tray icon (§3.4) | `sticon`, `taskbar`, `whichicon`, `whichicon2`, `minst` | name-based candidates only; none of them is established (all five are in S12 §5.2's list of keys the observed file did not carry) |
| agent keeps file types (§3.4) | `defext`, `extlist`, `check_ft_startup` | name-based candidates only; the file-type-related names in S12's list, and none of them is established (S12 §5.2) |
| visualization priority (§3.6) | `visplugin_priority`, `priority` | name-based candidates only; which of the two is the choice of §3.6 is open (§7, open question 6) |
| associate audio CDs (§3.1) | — | **no key of that name in S12 §5.1** |
| retake file types (§3.1) | — | **no key of that name in S12 §5.1** |
| metadata read timing (§3.2) | — | **no key of that name in S12 §5.1** |
| skin list (§3.3) | — | **no key of its own** — it is a list of what is available; the selected skin's file name is stored under `skin` (S12 §3.1) |
| skin directory (§3.3) | — | **no key of that name in S12 §5.1** |
| usage reporting (§3.4) | — | **no key of that name in S12 §5.1** |
| agent (§3.4) | — | **no key of that name in S12 §5.1** |
| QuickLaunch (§3.4) | — | **no key of that name in S12 §5.1** |
| visualization directory (§3.6) | — | **no key of that name in S12 §5.1** |
| DSP/effect directory (§3.6) | — | **no key of that name in S12 §5.1** |
| flush plug-in name cache (§3.6) | — | **no key of its own** — it is a button label, not a setting |

Counted from the table: **six** options match a key name outright, **five**
more have a candidate that rests on the name alone, **four** sit in a
count mismatch (two keys for one option, one key for three settings), and
**eleven** have no key of that name in S12's recognised set at all.

The other direction fails as well: S12 §5.1 recognises **109** key names, and
the great majority of them are not Preferences-page options — they are window
positions and sizes, equalizer values, playlist layout, minibrowser and video
geometry, analyser settings and plug-in selection. **Thirty** of them, plus
the six plug-in-selection keys, the observed file did not carry at all
(S12 §5.2). So the two sets are not the same set, and the useful statement
is the narrower one in §5: a setting that has a key **and** appears in the
option inventory of §3 is one we may be missing; a key outside that inventory
is not a gap until the option it belongs to is found.

## 5. Practical notes for an implementer

- **Our page tree is our own choice.** This spec fixes the **inventory**
  (§3) and the page labels (§1), not the layout, not the control types and
  not the wording of our pages. The reference's own dialog was measured at
  443 × 336 (§1.3); that is the reference's size, not a target.
- **Keep the help texts verbatim if we reuse them.** They are the
  reference's own user-facing sentences, and they name the option's meaning
  better than a label would.
- **Do not assume one key per option.** §4 shows two cases where the counts
  differ: one option with two keys (`newverchk`, `newverchk2`) and three
  settings with one key (`inet_mode`). S12 §10 item 3 leaves the range of
  every numeric key open, and that includes the two values of §3 that name a
  magnitude — the snap distance in pixels and the morph rate.
- **The names in §4 are the names in the file.** S12 §4.1's prefix rule
  governs the player's own lookups: the player looks its names up under a
  seven-character `config_` prefix, and the file carries the name without it.
  §4's names are the file's spellings (S12 §5.1).
- **Do not implement a morphing shuffle from this spec.** S15 §1's model is a
  permutation without replacement, and the morph rate's text is not a
  measurement (§3.2, §6 open question 11).
- **The plug-in families have their own pages**, and the library is not a
  page of this dialog at all (S20 §0, §1).
- **Everything in §6 is unknown.** A behaviour that §6 lists must not be
  implemented as if it were settled.

## 6. Open questions

Kept from the draft, in the draft's order:

1. **The tree's parentage** (§1.1) — which label is a page and which is a
   child. The repository's own records of the reference's Preferences window
   (§1.3) already bear on this; the measurement should start from them.
2. **Control types and layout** — checkbox / radio / edit / dropdown, their
   order, and the dialog's size. This needs the window (a screenshot), not
   the strings. The frame's size has been measured in another lane (§1.3);
   the control types have not.
3. **The video page's full texts** (truncated in the table).
4. **The remaining pages' option lists** — the strings give the *help texts*
   for the options whose text is long, but a page with short labels leaves
   no long string behind, so a page may carry options this spec does not
   list. (The example label "Always on top" is illustrative, not a
   verbatim reference label — the reference's own menu reads `Always On Top`,
   S04 §4.1 row 5.)
5. **The music library's settings** — the library is a plug-in (S20 §0, §1)
   and its own page is not in the player's table.
6. **Whether the priority choice (Idle/Normal/High) is a page-level option or
   a per-plug-in one.**
7. **The default value of every option** (S12 §10 item 8 already says the
   defaults were not read).

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

8. **The page set behind the labels** (§1.2) — which pages exist beyond the
   ten labels, and which of the five unlabelled groups of §3 are pages of the
   Preferences dialog at all.
9. **The encoding of the three Internet-connection settings** (§3.5) — how
   `inet_mode` carries the three states, and whether the three texts describe
   one choice or three.
10. **The two update-check keys** (§4) — which of `newverchk` and
    `newverchk2` the option of §3.4 writes, and what the second one is for.
11. **The shuffle morph** (§3.2) — whether the reference's shuffle order
    drifts over time at all, and what the morph rate does. S15 §1's
    permutation model and the help text of §3.2 disagree; the measurement
    that settles it is named in §3.2.
