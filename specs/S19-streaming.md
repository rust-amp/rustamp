---
Spec ID: S-19
Title: Streaming and ICY metadata display
Sources consulted (category only): dirty room: string table of the reference binary; dirty room: a configuration file written by the reference
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S19 — streaming and ICY metadata display

## 0. How to read this spec

**No stream was played for this spec, and the reference binary was not run.**
Everything below is read out of the reference's own **user-facing status
strings** and out of the **HTTP request template** it carries; the fourth
artifact is the configuration file the reference wrote, which is what the
proxy setting is cross-checked against. No behaviour was observed at runtime,
so nothing here is a measurement of a running player. Where a statement goes
beyond what a string or a template can carry on its own, it is marked
**reading** — a hypothesis, never a fact to implement.

| Where it comes from | What it means |
|---|---|
| §1 | **Template fact** — what the reference's own request template says: the request line, the protocol version, the header names it carries, the number of placeholder slots it has. |
| §2 | **String fact** — a spelling the reference carries: a status text, or a format string with the placeholders it counts. |
| §3 | **Field fact** — a field name the title-format language carries (S14), and a **reading** about what the reference does with it. |
| §4 | **Config fact** — what the player's recognised key set carries about the proxy setting, and what the observed configuration file carried. |
| §5 | **Practical note** — a consequence that follows from §1–§4; nothing new. |
| §6 | **Open** — not established here; the measurement that would settle it is named. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing. What the audit
changed, and what it found, is in `docs/notes/spec-audit-s19.md`.

**Scope.** This spec is about the player's HTTP streaming path: the request it
sends, the status texts it shows while a stream runs, the proxy setting and
the stream-title field. It does **not** cover how a stream is decoded or
buffered, and it does not settle the seek bar, the time display or the output
path — S18 owns output behaviour, and no stream was played here. The two
behavioural conclusions of §2 are readings, not observations.

## 1. The request the player sends

The reference's own string table carries an HTTP request template. Rendered
with its placeholders filled in, it reads:

```
GET http://<host>:<port>/<path>
HTTP/1.0
Host: <host>
User-Agent: Winamp/<version>
<extra headers>
Accept: */*
```

Facts about the template:

- The request line carries the **absolute** form: scheme, host, port and path
  in one URI, not a path alone.
- The protocol line names **HTTP/1.0**.
- The template carries a `Host:` line, a `User-Agent:` line and an
  `Accept: */*` line.
- The `User-Agent` names the player and carries a **version**. The version
  string itself is not established here.
- There is **one slot for extra header lines**, between the `User-Agent` line
  and the `Accept` line, and the template has **two placeholder slots** in it.
  What the slots carry is open (§6, open question 9).

Readings:

- The request asks for no persistent connection: the template carries no
  `Connection:` line, and HTTP/1.0 is not a keep-alive protocol by default.
  (reading)
- The absolute form in the request line is the form a proxy receives, so the
  one template serves the proxy path as well (§4). (reading)
- The template's own header lines carry no `Range:` line, so a seek is not
  negotiated with the server through them. (reading — and the two
  extra-header slots are open, so this is not settled: §6, open questions 9
  and 11)
- A response to an HTTP/1.0 request is not chunked. (reading — it follows
  from the protocol version, not from the template)

## 2. The status texts

| Phase the text names | Text |
|---|---|
| before connecting | `Connecting to host...` |
| proxy path | `Connecting to proxy` |
| request sent | `Sending HTTP request` |
| response read | `Reading HTTP response` |
| transfer, total unknown | `Retrieving file...`, then `%d bytes read @ %d.%02d KB/sec` |
| transfer, total known | `Retrieving file (%d%% done)`, then `%d/%d bytes read @ %d.%02d KB/sec` |
| host vanished | `Connection to host lost` |
| server closed | a leading newline, then `Connection closed.` |
| connect failed | `Error connecting to host` |
| proxy failed | `Error connecting to proxy` |

Facts about the texts:

- The table's **ten** entries carry **twelve** texts: the two transfer entries
  are two-line shapes and therefore hold two texts each, the other eight
  entries hold one.
- The server-closed text carries a **leading newline**; it is not a bare
  sentence.
- Each counters line names the byte count and the rate; the second one also
  names the ratio of read bytes to total bytes and a percentage.
- The rate is written with **two decimals**, in KB/s.
- The percentage text is spelled `(%d%% done)` — the doubled percent sign is
  how the format spells one literal percent sign.

The phase column groups the texts by what each one names. **The order in which
the player shows them was not observed** (§6, open question 15).

Two conclusions follow from the two transfer texts. Both are **readings**,
because no stream was played:

- **The total is optional.** The two texts are the same display in two
  variants: the first for a response that carries no length, the second for a
  response that does. A live stream and a finite file therefore take the two
  different texts, and the difference between them is exactly the total.
- **The counters are a progress display, not a summary.** The counters line
  is written while the transfer runs, not once at the end: it carries the
  running byte count and the running rate, which is what a display that
  advances during the transfer looks like.

## 3. Metadata on the stream

Facts:

- The title-format language's field list includes **`streamtitle`**. S14 §2.1
  lists it among the **nine** field names the material carries, and S14 §2.4's
  help-text illustration uses it in a format string.
- That language belongs to **individual input plug-ins**, not to the player
  (S14 §1): the reference has no global title-format setting, and the
  player's own strings carry no field token of that language.

The step from the field name to "the reference reads the ICY metadata block
and its `StreamTitle` entry" is a **reading**, and it sits in tension with
S14 §1: the field list belongs to a plug-in's format language, and the
player's own strings carry no such token. Nothing in this spec's own
material shows that the reference parses an ICY metadata block at all —
published as open question 12.

What the reference does with a stream title **outside** the format language —
the main window's marquee, a playlist row — is open (open question 6).

## 4. Proxy

Facts:

- The reference carries **two proxy-specific status texts**:
  `Connecting to proxy` and `Error connecting to proxy`. The proxy path
  therefore has its own connect phase and its own connect failure, distinct
  from the direct ones (`Connecting to host...`, `Error connecting to host`).
- The player recognises a **`proxy`** setting: `proxy` is in the recognised
  key set of the `[Winamp]` section (S12 §5.1). The file spells the key
  **without** the reader's seven-character `config_` prefix (S12 §4.1), so
  the name in the file is `proxy`.
- The configuration file the reference wrote **did not carry that key**
  (S12 §5.2). No proxy string was therefore observed: neither a value, nor
  its shape, nor whether the reference writes one at all.

Reading:

- The absolute request line of §1 is the form a proxy receives, so the same
  template serves both paths.

Open:

- Whether the proxy path and a direct connection use the same code path
  after the connect is not established.
- The shape of the proxy setting — host, port, credentials, scheme — is not
  established either (open question 13).

## 5. Practical notes for an implementer

- The request to send is the one of §1, in the absolute form, whether the
  connection goes through a proxy or not (§4).
- The status texts of §2 are the reference's own user-facing strings. Keep
  their spelling, including the leading newline of `Connection closed.` and
  the two-line shape of the two transfer texts.
- The transfer display has **two** variants; pick by whether the response
  carries a length (§2).
- The counters line is written while the transfer runs (§2, reading).
- The proxy setting is `proxy` in the file, and the reference wrote none in
  the observed run — so it has to come from the user, and its format is open
  (§6, open question 13).
- Do **not** treat the `streamtitle` field as proof that the reference parses
  ICY metadata (§3, open question 12).
- Everything in §6 is unknown. A behaviour that §6 lists must not be
  implemented as if it were settled.

## 6. Open questions

Kept from the draft, in the draft's order. Items 1, 3 and 4 were re-labelled:
the absence of a text is a fact about the **string table**, not a conclusion
about the behaviour.

1. **Timeouts** — the reference's string table carries no timeout text. That
   is a fact about the **table**: whether a stalled host blocks or is cut by a
   socket default is a **reading**, the value is not established, and whether a
   timeout exists at all is open question 14.
2. **Retry count** on a failed connect.
3. **The buffering policy** — when playback starts, how much is buffered, and
   whether a mid-stream stall is reported. The string table carries no stall
   text either, so a stall is likely to be silent; that is a reading, not a
   fact (the same silence as in question 1, and open question 14 asks what
   such a silence is worth).
4. **Status codes** — the string table carries no dedicated text for 404 /
   403 / 500; whether the body is played as if it were audio, or a generic
   error is shown, is open.
5. **The `Icy-MetaData` request header** — whether the player asks for
   metadata explicitly or only reacts when the server sends it.
6. **Where the stream title is displayed** (marquee, playlist, title bar).
7. **Whether a stream seek is possible at all**, and what the seek bar shows
   without a length.
8. **`http://` only, or other schemes** — the template is plain `http://`, so
   there is no evidence for `https`; whether a `https` URL is handled is
   open. (Background, not a measurement: 2.95 predates the common use of TLS
   streams.)
9. **The exact `<extra headers>`** the template's two placeholder slots carry.
10. **Whether the `Host:` header includes the port** for a non-default port.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

11. **Whether the request ever carries a `Range` header.** §1's "no `Range`
    request" and open question 9 contradict each other: the template's own
    header lines carry no `Range:` line, but the two extra-header slots are
    open. Settled by capturing one request against a server that logs the
    headers it receives.
12. **Whether the reference reads an ICY metadata block at all**, and what
    feeds the `streamtitle` field. S14 §2.1's field list is a plug-in's, and
    the player's own strings carry no field token (S14 §1). A capture against
    a server that sends ICY metadata would settle it.
13. **The shape of the proxy setting.** The observed configuration file
    carried no value for it (S12 §5.2), so the format — host, port,
    credentials, scheme — is not established, and neither is whether the
    reference writes the key.
14. **Whether a timeout exists at all**, and whether the string table's
    silence settles it. A string table without a timeout text does not decide
    whether a timeout is applied; the value could be silent. (The same
    question S14 asks about a token silence in its open question 9.)
15. **The order in which the status texts appear**, and how many requests the
    player makes per stream. §2 groups the texts by the phase each names; the
    sequence was not observed, and the template shows one request's shape, not
    the number of requests.
16. **Whether the template's header lines are always sent exactly as shown** —
    in particular whether `Accept: */*` is always present, as the draft
    asserted, and whether the `Host:` line is emitted on the proxy path.
