# W19 — Adversariales Review der neuesten Wave (Wave 92)

**Datum:** 2026-10-18  
**Autor:** Antigravity (Adversarial Reviewer)  
**Arbeitsverzeichnis:** `paseo-wave92-review`  
**Prüfungsregeln:** Reines Review, keine Codeänderungen, kein Commit, kein `cargo test`, keine VM-Ausführung.  
**Erlaubte Prüfgates:**  
- `cargo fmt --all -- --check`: **Exit 0** (bestanden)  
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **Exit 0** (bestanden)  
- `cargo build --workspace`: **Exit 0** (bestanden)

---

## 1. Claim-Tabelle

| Gegenstand / Task-Card | Behauptung der Card | Verdikt | Evidenz (Datei:Zeile) | Kleinste Reproduktion |
|---|---|---|---|---|
| **W17-vis-clear** (`docs/tasks/W17-vis-clear.md`) | Der Visualisierer leert den Bildschirm sofort bei Stop in jedem Modus (`Spectrum`, `Scope`, `ScopeDots`); der Test beweist dies statt auszuweichen. | **Teils widerlegt** (Code leert, Test spart Modi aus) | `crates/rustamp-ui/src/visualization.rs:247-254`, `crates/rustamp-app/src/visualization_tests.rs:201` | `visualization_tests.rs:201`: Die Schleife iteriert nur über `[VisMode::Spectrum, VisMode::Scope]`. `VisMode::ScopeDots` und `VisMode::Off` werden im Test vollständig ausgelassen. |
| **W18-window-focus** (`docs/tasks/W18-window-focus.md`) | Der Fokuszyklus überspringt geschlossene Fenster; `Minimize` und `ShowMainWindow` lösen strikt so auf, dass der letzte Befehl in beiden Reihenfolgen gewinnt. | **Bestätigt** (im Modell) / **Einschränkung** (bei Minimierung) | `crates/rustamp-app/src/focus.rs:43-55`, `crates/rustamp-app/src/app.rs:826-846, 1440-1448`, `crates/rustamp-app/src/window.rs:402-418` | `App::focus_next_window`: Wenn `Main` minimiert ist, bleibt `main_visible == true`. `next_focus` wählt `Main` aus; `apply_focus_requests()` ruft `window.focus_window()` auf, ohne `set_minimized(false)` zu rufen (da `restore == false`). |
| **W17-xtask-findings** (`docs/tasks/W17-xtask-findings.md`) | Die ausgelieferten Lizenztexte erfüllen alle Distributor-Pflichten für MPL-2.0 und Apache-2.0 und sind kanonisch; die README nennt Visualisierer und Shuffle nicht mehr Placebos. | **Teils widerlegt** (Player-Notices vollständig, Host-Notices gekürzt) | `xtask/licenses/MPL-2.0.txt:1-373`, `LICENSE-APACHE:1-203`, `xtask/src/package.rs:1188-1205, 1340, 1445-1457` | `package.rs:1445-1457`: `host_third_party_notices()` enthält unter `### Apache License, Version 2.0` nur den 14-zeiligen Standard-Header/Notice-Text, nicht die Lizenzbedingungen (wird durch Beilage von `host/LICENSE-APACHE` im Zip gemildert). README-Placebo-Texte sind entfernt. |
| **W17-xtask-findings** / PE-Imports (`xtask/src/pe_imports.rs`) | Der PE-Import-Inspektor verweigert Binärdateien mit ungültigen/fremden Imports strikt und lässt nur echte Windows-System-DLLs zu, bevor das Manifest-Flag gesetzt werden darf. | **Widerlegt** (Bypass & unzureichende Validierung) | `xtask/src/pe_imports.rs:57-69, 198-212`, `xtask/src/package.rs:404` | 1. DLL-Namen wie `api-ms-win-evil.dll` oder `ext-ms-win-crt-*.dll` passieren `is_system_dll()`. 2. Ein PE mit `imp_rva != 0 && imp_size != 0`, dessen erster Import-Deskriptor nulliert ist, bricht mit 0 Modulen ab; `check_modules` prüft eine leere Liste, meldet `Ok(())` und setzt `artifact_imports_verified = true`. |
| **W18-golden-bless-path** (`docs/tasks/W18-golden-bless-path.md`) | Das Bless-Token leckt niemals in den Binärnamen-Filter und blesset niemals ungefragt. | **Widerlegt** (Token-Leak & Massen-Blessing nachgewiesen) | `docs/notes/vm-broker/guest/run-tests.ps1:44-55, 233`, `docs/notes/vm-run-tests.ps1:348` | 1. `-Filter '--ignored:--bless:golden'`: `$filter -like '--bless*'` schlägt fehl. Das Strippen von `--ignored:` belässt `$filter = '--bless:golden'`, welches ungestrippt in Zeile 233 leckt (0 Tests laufen). 2. `-Filter '--bless'` leert `$filter`, wodurch ALLE Test-Binärdateien mit `RUSTAMP_BLESS=1` laufen. |
| **W18-prev-cycle-red** (`docs/tasks/W18-prev-cycle-red.md`) | Der korrigierte Zyklustest `shuffle_repeat_prev_across_cycle_boundary` testet weiterhin das Verhalten, nach dem er benannt ist (S15 §3.2 Prev über Zyklusgrenze). | **Bestätigt** | `crates/rustamp-audio/tests/shuffle_repeat.rs:358-410`, `crates/rustamp-audio/src/engine.rs:1081-1090` | Der Test startet am Anfang von Zyklus 1 (`plan1[0]`), durchläuft Zyklus 1, überschreitet die Zyklusgrenze zu Zyklus 2 Track 0 (`plan2[0]`), drückt `prev_track()` und verifiziert den Rücksprung zu `plan1[2]` sowie die Wiederherstellung von `plan1`. |

---

## 2. Befunde in Schweregrad-Reihenfolge

### [W19-F01] Nachgewiesen: Bless-Token leckt in Binärnamen-Filter bei kombinierten Parametern
- **Schweregrad:** Hoch (Funktionsverlust / Runner-Defekt)
- **Status:** Nachgewiesen (*nachgewiesen*)
- **Betroffene Dateien:** `docs/notes/vm-broker/guest/run-tests.ps1:44-55, 233`
- **Beschreibung:**  
  In `docs/notes/vm-broker/guest/run-tests.ps1` wird die Token-Prüfung linear durchgeführt:
  ```powershell
  $bless = $false
  if ($filter -like '--bless*') {
      $bless = $true
      $filter = ($filter -replace '^--bless:?', '')
      $env:RUSTAMP_BLESS = '1'
  }
  if ($filter -like '--ignored*') {
      $extra = @('--ignored', '--test-threads=1')
      $filter = ($filter -replace '^--ignored:?', '')
  }
  ```
  Wird der Test-Runner mit `-Filter '--ignored:--bless:golden'` aufgerufen (um ignorierte Tests mit Re-Blessing auszuführen), ist `$filter -like '--bless*'` falsch. Anschließend entfernt der zweite Block `--ignored:`, wodurch `$filter` exakt zu `'--bless:golden'` wird. Da kein weiterer Bless-Parsing-Schritt existiert, erreicht dieser String unverändert Zeile 233:
  ```powershell
  if ($filter -and $name -notlike "*$filter*") { continue }
  ```
  Da kein Test-Executable den Substring `--bless:golden` im Dateinamen führt, überspringt der Runner ausnahmslos alle Binärdateien. Das Ergebnis ist ein fehlschlagender Durchlauf mit 0 ausgeführten Tests (`ran: 0`), und `$bless` bleibt `false`.

---

### [W19-F02] Nachgewiesen: Unbeabsichtigtes Blessing aller Tests bei unvollständigem `--bless` und Wildcard-Fehlgriffen
- **Schweregrad:** Hoch (Datenkorruption im Test-Gast)
- **Status:** Nachgewiesen (*nachgewiesen*)
- **Betroffene Dateien:** `docs/notes/vm-broker/guest/run-tests.ps1:44-50`, `docs/notes/vm-run-tests.ps1:348`
- **Beschreibung:**  
  1. **Massen-Blessing:** Übergibt der Benutzer `-Filter '--bless'` (ohne Suffix `:name`), matcht `$filter -like '--bless*'`. Der Regex `$filter -replace '^--bless:?', ''` ergibt den leeren String `''`. In Zeile 233 ist `$filter` leer, sodass der Filter für sämtliche Binärdateien deaktiviert ist. Als Folge werden alle 34+ Test-Binärdateien mit `$env:RUSTAMP_BLESS = '1'` ausgeführt. Jedes Golden-Asset in der Suite wird im Gast ungefragt überschrieben.
  2. **Präfix-Kollision:** Durch den Wildcard-Vergleich `$filter -like '--bless*'` lösen auch unabsichtliche Filter wie `-Filter '--bless-suite'` oder `-Filter '--blessing'` die Bedingung aus. Bei `--bless-suite` wird nur `--bless` gestrippt und `$filter` verbleibt als `'-suite'`, während `$env:RUSTAMP_BLESS = '1'` scharfgeschaltet wird.

---

### [W19-F03] Nachgewiesen: PE-Import-Inspektor akzeptiert fremde DLLs und leere Import-Tabellen
- **Schweregrad:** Mittel (Sicherheits- und Packaging-Validierungslücke)
- **Status:** Nachgewiesen (*nachgewiesen*)
- **Betroffene Dateien:** `xtask/src/pe_imports.rs:57-69, 198-212`, `xtask/src/package.rs:404`
- **Beschreibung:**  
  Die Card `W17-xtask-findings.md` und der Modul-Docstring versprechen, dass nur Windows-System-DLLs akzeptiert werden und fehlerhafte oder leere Importtabellen strikt abgewiesen werden. Die Implementierung weist jedoch zwei strukturelle Lücken auf:
  1. **Generischer API-Set-Bypass:**  
     ```rust
     if (lower.starts_with("api-ms-win-") || lower.starts_with("ext-ms-win-"))
         && lower.ends_with(".dll")
     {
         return true;
     }
     ```
     `is_system_dll()` filtert lediglich `api-ms-win-crt-`. Ein hypothetisches CRT- oder Drittmodul mit dem Präfix `ext-ms-win-crt-*.dll` oder jede beliebige bösartige/private DLL mit dem Namen `api-ms-win-foo.dll` wird bedingungslos als System-DLL eingestuft und passiert den Test.
  2. **Bypass bei nulliertem ersten Deskriptor:**  
     In `parse_imports()` wird geprüft: `if imp_rva == 0 || imp_size == 0 { return Err(...); }`. Enthält die Tabelle jedoch einen sofortigen Null-Terminator (`name_rva == 0 && first_thunk == 0 && original == 0`), bricht die Schleife ab und liefert `imports.imports` mit 0 Elementen zurück. In `check_modules()` wird über `imports.all()` iteriert; da die Liste leer ist, bleibt `bad` leer und die Funktion gibt `Ok(())` zurück. `inspect_imports()` akzeptiert die Binärdatei und meldet `imports OK ... 0 module(s)`, woraufhin `artifact_imports_verified: true` im Manifest gesetzt wird.

---

### [W19-F04] Nachgewiesen: Fokus-Kommando `FocusNextWindow` un-minimiert minimiertes Hauptfenster nicht
- **Schweregrad:** Mittel (UI/Fokus-Inkonsistenz)
- **Status:** Nachgewiesen (*nachgewiesen*)
- **Betroffene Dateien:** `crates/rustamp-app/src/app.rs:608-616, 826-834`, `crates/rustamp-app/src/window.rs:402-418`
- **Beschreibung:**  
  In `App::window_open(WindowId::Main)` wird ausschließlich `self.main_visible` abgefragt. Ein Minimieren des Hauptfensters über `Command::Minimize` setzt jedoch nicht `main_visible = false`, sondern lediglich das transiente Flag `minimize_requested = true`.  
  Wird das Hauptfenster minimiert und betätigt der Benutzer anschließend `FocusNextWindow` (Ctrl+Tab), meldet `window_open(WindowId::Main)` weiterhin `true`. `App::focus_next_window()` setzt `focus_request = Some(WindowId::Main)`.  
  Im Presenter (`apply_focus_requests()` in `window.rs:402-418`) ist `restore` jedoch `false`, da `restore_request` nur von `ShowMainWindow` gesetzt wird. Der Presenter ruft daher lediglich `window.focus_window()` auf dem minimierten Fenster auf. Unter Windows führt dies dazu, dass das minimierte Fenster in der Taskleiste blinkt oder den Fokus im minimierten Zustand erhält, anstatt restored (un-minimiert) zu werden.

---

### [W19-F05] Nachgewiesen: Visualisierer-Tests lassen Modi `ScopeDots` und `Off` ungetestet
- **Schweregrad:** Niedrig (Testlücke bei bestehender Code-Korrektheit)
- **Status:** Nachgewiesen (*nachgewiesen*)
- **Betroffene Dateien:** `crates/rustamp-app/src/visualization_tests.rs:201`, `crates/rustamp-ui/src/visualization.rs:593`
- **Beschreibung:**  
  Die Card `W17-vis-clear.md` deklariert:  
  *"Screen clearing upon stop must apply uniformly across all visualization modes (`Spectrum`, `Scope`, `ScopeDots`)."*  
  Im Code leert `advance()` bei `TransportState::Stopped` durch `*self = Self::default()` alle Puffer. In `draw_visualization()` greift bei `VisMode::ScopeDots` der Guard `if state.input == VisInput::Fresh`; da `input` bei `Default` auf `VisInput::Missing` gesetzt ist, wird das Oszilloskop auch bei `ScopeDots` nicht gezeichnet.  
  **Die Tests beweisen dies jedoch nicht vollständig:** Sowohl `visualization_tests.rs:201` (`stopping_playing_or_paused_cannot_retain_scope_or_stale_bars`) als auch `visualization.rs:593` (`stop_and_reset_do_not_replay_scope_or_retain_old_rate_envelopes`) iterieren ausschließlich über:
  ```rust
  for mode in [VisMode::Spectrum, VisMode::Scope] {
  ```
  `VisMode::ScopeDots` und `VisMode::Off` fehlen in beiden Schleifen. Der frühere Betrug (Ausweichen von Spectrum auf Scope kurz vor Stop) wurde zwar behoben, die Testabdeckung ist jedoch unvollständig geblieben.

---

### [W19-F06] Nachgewiesen: Unvollständiger Apache-2.0-Text in Standalone Plugin Host Notices
- **Schweregrad:** Niedrig (Lizenzhinweis-Inkonsistenz)
- **Status:** Nachgewiesen (*nachgewiesen*)
- **Betroffene Dateien:** `xtask/src/package.rs:1445-1457`
- **Beschreibung:**  
  Während `player_third_party_notices()` den vollständigen kanonischen 203-zeiligen Text von `LICENSE-APACHE` einbettet, enthält `host_third_party_notices()` in `xtask/src/package.rs` unter `### Apache License, Version 2.0` lediglich den 14-zeiligen Standard-Notice-Header (`Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file except in compliance...`).  
  Zwar legt das Packaging-Skript die separate Datei `host/LICENSE-APACHE` neben die Notice-Datei ins Zip-Archiv, die in `host/THIRD_PARTY.md` versprochene Lizenztext-Vollständigkeit weicht jedoch von den Player-Notices ab.

---

### [W19-F07] Vermutet: Winit-Fokusübergabe unterliegt Windows Foreground Lock Restriction
- **Schweregrad:** Niedrig / Plattformbedingt
- **Status:** Vermutet (*vermutet* — ohne echten Desktop nicht widerlegbar)
- **Betroffene Dateien:** `crates/rustamp-app/src/window.rs:416`, `docs/tasks/W18-window-focus.md`
- **Beschreibung:**  
  `Window::focus_window()` ruft winits interne Plattformfunktion `SetForegroundWindow` bzw. `SetFocus` auf. Windows blockiert unaufgeforderte Fokusübernahmen von Hintergrundprozessen (`LockSetForegroundWindow`). Wenn Rustamp nicht das aktive Vordergrundfenster ist, führt `focus_window()` lediglich zu einem Blinken des Taskleisteneintrags. Die App aktualisiert ihren internen Zustand `focused_window` optimistisch, ohne Feedback vom OS (`WindowEvent::Focused(true)`) abzuwarten.

---

## 3. Was ohne VM oder echten Desktop nicht verifiziert werden kann

Folgende Aspekte sind im reinen Headless-Worktree prinzipbedingt unentscheidbar und bedürfen einer realen Windows-Desktop-Umgebung oder der Hyper-V-Test-VM:

1. **Reale Windows-Fokus- und Aktivierungssemantik:**
   - Ob Windows das Tastatur- und Eingabefokus-Handle tatsächlich an das über `window.focus_window()` angeforderte Fenster übergibt (insbesondere bei Multi-Window mit Equalizer und Playlist).
   - Ob `window.set_minimized(false)` ein minimiertes Fenster aus dem Taskleisten-Zustand verlässlich restauriert und an die Spitze der Z-Order bringt, wenn das Tray-Icon zuvor aktiv war.
2. **Laufzeitverhalten des Hyper-V Guest Broker Bless-Pfades:**
   - Das Zusammenspiel zwischen Host-Skript `vm-run-tests.ps1`, Broker `invoke-broker.ps1` und Gast-Skript `run-tests.ps1` bei der Ausführung von `copyout` aus `C:\rustamp\out\bless` nach `staging\out\bless-golden`.
   - Ob die generierten Golden-PNGs pixelidentisch zu den erwarteten Referenzen unter realer Softbuffer-/GDI-Rasterung gerendert werden.
3. **PE-Ladeverhalten auf Clean-Machine-Systemen:**
   - Der PE-Import-Inspektor prüft nur statische Importtabellen. Ob eine Binärdatei zur Laufzeit dynamisch via `LoadLibrary` oder delay-load Bibliotheken nachlädt, die auf einem minimalen Windows 10/11 fehlen, kann nur durch realen Start auf einer echten Clean-Machine ohne installierte Entwickler-Runtimes verifiziert werden.
4. **Physische Audio-Endpunkt- und Latenz-Reaktion:**
   - Das Verhalten des Audio-Pumps bei schnellen Transportwechseln (`Stop` -> `Play` -> `Seek`) und nahtlosem Zykluswechsel (`Next` / `Prev` an der Playlist-Grenze) auf realen WASAPI-Endpunkten im Vergleich zur Test-Clock des Harness.
