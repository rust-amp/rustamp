# Adversarielles Review: Wave S15, S08, X10, F5 (Rustamp)

Datum: 2026-10-18  
Prüfer: Antigravity (DeepMind Agent)  
Arbeitsverzeichnis: Paseo Worktree `paseo-wave-review`  
Geprüfte Spezifikationen und Komponenten:
- **Shuffle & Repeat:** `specs/S15-shuffle-repeat.md`, `docs/tasks/S15-shuffle-repeat-impl.md`, `crates/rustamp-playlist`, `crates/rustamp-audio`
- **Visualisierer:** `docs/tasks/S08-vis-impl.md`, `docs/tasks/S08-vis-wiring.md`, `crates/rustamp-ui/src/visualization.rs`, `crates/rustamp-audio/src/vistap.rs`, `crates/rustamp-app/src/app.rs`
- **Release-Archiv & Lizenzinventar:** `docs/tasks/X10-release-zip.md`, `xtask/src/package.rs`, `THIRD_PARTY.md`
- **Tray-Vervollständigung:** `docs/tasks/F5-tray-seams.md`, `crates/rustamp-platform/src/tray.rs`, `crates/rustamp-app/src/window.rs`

---

## 1. Übersicht & Kernaussagen zur Falsifizierung

| Fragestellung der Prüfung | Ergebnis | Zusammenfassung |
|---|---|---|
| **Überlebt die Shuffle-Reihenfolge ein manuelles Vorrücken (Next)?** | **Nein (Widerlegt)** | Divergenz zwischen `Engine` und `Worker`: Beim Start mutiert `self.shuffle_seed`, wodurch der `Worker` mit einem abweichenden Shuffle-Plan decodiert als `Engine::shuffle_plan()` ausgibt. |
| **Überlebt die Shuffle-Reihenfolge ein Umschalten (Toggle) mid-run?** | **Nein (Widerlegt)** | Beim Re-Aktivieren von Shuffle mitten im Durchlauf werden bereits gehörte Titel wieder in den Shuffle-Pool aufgenommen (`replan_shuffle_mid_run` ruft `init_shuffle_plan(current)` auf). |
| **Überlebt die Shuffle-Reihenfolge das Warteschlangenende (Queue End)?** | **Nein (Widerlegt)** | Bei `repeat = false` stoppt der Worker am Queue-Ende, aber `Engine.shuffle_pos` verbleibt auf 0. Ein manuelles `next_track()` im Stop-Zustand startet Titel `shuffle_plan[1]`, statt gestoppt zu bleiben. |
| **Fabriziert der Visualisierer wirklich keine Samples?** | **Teilweise widerlegt** | `decode.rs` bügelt nicht-finite Samples (`NaN`, `Inf`) vor dem `VisTap` in-place zu `0.0` platt. Der Visualisierer sieht dadurch synthetische Stille statt des spezifizierten Fehlers `VisError::NonFinitePcm`. Mono wird zudem auf 2 Kanäle dupliziert. |
| **Löscht der Visualisierer nach einem Stop wirklich den Bildschirm?** | **Nein (Widerlegt)** | Im Spektrum-Modus (`VisMode::Spectrum`) verbleiben Balken und Peaks bis zu 1.300 ms sichtbar und klingen nur allmählich ab; der Bildschirm wird bei Stop nicht sofort gelöscht. |
| **Liefert das Lizenzinventar wirklich alles, was die Lizenzen fordern?** | **Nein (Widerlegt)** | Volltext der MPL-2.0 für Symphonia fehlt in `player/THIRD_PARTY.md` trotz gegenteiliger Behauptung; Apache-2.0 enthält nur den kurzen Header; die Release-README enthält veraltete Disclaimers (Placebo-Status). |
| **Verweigert die Layout-Prüfung weiterhin alles, was sie muss?** | **Ja (Bestätigt)** | Die strikte Whitelist in `audit_package_layout` und die GPL/Copyleft-Prüfungen in `audit_host_dir` verweigern unerwartete oder unzulässige Dateien zuverlässig. |
| **Leitet das Tray jede Geste über einen einzigen Command-Pfad?** | **Ja (Bestätigt)** | Linksklick, Doppelklick und Kontextmenü münden ausnahmslos über `submit_platform_commands()` direkt in `App::submit()`. |

---

## 2. Behauptungstabelle (Claim Table)

| Behauptung | Urteil | Beleg Datei:Zeile | Kleinste Reproduktion |
|---|---|---|---|
| **S15-C1:** `rustamp-playlist/src/order.rs` implementiert die verbindliche Shuffle-Ordnung nach S15 für die Audio-Engine. | **Widerlegt** | `crates/rustamp-playlist/src/order.rs:45-310`, `crates/rustamp-audio/Cargo.toml` | `git grep PlaybackOrder crates/rustamp-audio crates/rustamp-app` liefert 0 Treffer. `PlaybackOrder` ist toter Code; `rustamp-audio` reimplementiert die Logik unabhängig und fehlerhaft in `engine.rs` und `decode.rs`. |
| **S15-C2:** Die von `Engine::shuffle_plan()` exponierte Permutation entspricht der tatsächlichen Abspielreihenfolge des Audio-Decoders (`Worker`). | **Widerlegt** | `crates/rustamp-audio/src/engine.rs:633-650`, `engine.rs:1523-1533`, `crates/rustamp-audio/src/decode.rs:704-716` | Engine mit Seed 42 und Queue `[0, 1, 2, 3]` initialisieren; `engine.play()` aufrufen. `engine.shuffle_plan()` liefert Permutation $P_A$ (berechnet mit Seed 42), während `Worker` mit Seed 43 gestartet wird und Permutation $P_B$ decodiert. |
| **S15-C3:** Nach Erreichen des Warteschlangenendes bei `repeat = false` verhindert das System weiteres Abspielen bei manuellem Next. | **Widerlegt** | `crates/rustamp-audio/src/decode.rs:575-578`, `crates/rustamp-audio/src/engine.rs:966-978`, `engine.rs:1367-1372` | Queue mit 3 Titeln, `shuffle=true, repeat=false`. Queue natürlich bis zum Stop auslaufen lassen. `engine.next_track()` aufrufen: `self.shuffle_pos` ist 0, Bedingung `0 + 1 < 3` greift, Titel `shuffle_plan[1]` wird abgespielt. |
| **S15-C4:** `Prev` im Shuffle + Repeat am Zyklus-Anfang (`pos == 0`) springt zum Ende des *vorherigen* Zyklus (S15 §3.2). | **Widerlegt** | `crates/rustamp-audio/src/decode.rs:579-581`, `crates/rustamp-audio/src/decode.rs:604-607` | Queue `[0, 1, 2]`, `shuffle=true, repeat=true`. Am Ende von Zyklus 1 generiert `replan_shuffle_new_cycle` Zyklus 2 und überschreibt `self.shuffle_plan`. Bei `pos == 0` `prev_track()` aufrufen: springt zu Index `len - 1` von Zyklus 2 statt Zyklus 1. |
| **S15-C5:** Mid-Run Toggle von Shuffle behält bereits gespielte Titel im aktuellen Durchlauf im Ausschluss (S15 §4 Anti-Wiederholung). | **Widerlegt** | `crates/rustamp-audio/src/decode.rs:722-724`, `decode.rs:698-705` | Queue `[0, 1, 2, 3]`. Titel 0 und 1 linear abspielen. Shuffle einschalten. `replan_shuffle_mid_run(1)` ruft `init_shuffle_plan(1)` auf, welcher `(0..4).filter(\|&i\| i != 1)` poolt. Titel 0 ist erneut im Pool und kann sofort nach 1 wiederholt werden. |
| **S08-C1:** Der Visualisierer fabriziert niemals synthetische Audio-Samples aus fehlerhaftem/fehlendem PCM (S08 §1). | **Teilweise widerlegt** | `crates/rustamp-audio/src/decode.rs:928-932`, `crates/rustamp-ui/src/visualization.rs:136-138` | `Worker::emit_slice` mutiert `in_buf` vor Übergabe an `VisTap`: Nicht-finite Werte (`f32::NAN`) werden mit `0.0` überschrieben. `VisError::NonFinitePcm` im UI-Visualisierer kann über den Audio-Pfad niemals ausgelöst werden. |
| **S08-C2:** Nach `Command::Stop` wird der Bildschirminhalt des Visualisierers sofort bereinigt (Clear Screen). | **Widerlegt** | `crates/rustamp-ui/src/visualization.rs:264-294`, `visualization.rs:388-410`, `crates/rustamp-app/src/visualization_tests.rs:210-226` | Im Modus `VisMode::Spectrum` bei vollem Spektrum `app.submit(Command::Stop)` aufrufen und sofort rendern (`now_ms == stop_ms`): Balken und Peak-Marker bleiben bis zu 1.300 ms sichtbar auf dem Bildschirm gezeichnet. |
| **X10-C1:** Das Release-Artefakt liefert den vollständigen Lizenztext der MPL-2.0 für Symphonia (X10 Anforderung). | **Widerlegt** | `xtask/src/package.rs:1172`, `xtask/src/package.rs:1220-1234` | In `player_third_party_notices()` deklariert Zeile 1172: "Symphonia MPL-2.0 notice, source availability, and full license text". Abschnitt 4 enthält jedoch nur 14 Zeilen Zusammenfassung und eine Web-URL; der Lizenztext fehlt. |
| **X10-C2:** Das generierte `README.md` im Release-Paket beschreibt den aktuellen Funktionsstand der Wave wahrheitsgemäß. | **Widerlegt** | `xtask/src/package.rs:1133` | `xtask package` generiert `README.md` mit Zeile 1133: *"Spectrum analyzer is contracted to zero. Shuffle and repeat UI toggles are not yet connected to the playback engine."* Dies widerspricht der implementierten Wave direkt. |
| **X10-C3:** Die Layout-Prüfung (`audit_package_layout`, `audit_host_dir`) lehnt unerlaubte Dateien und falsche Lizenzen ab. | **Bestätigt** | `xtask/src/package.rs:723-748`, `xtask/src/package.rs:1490-1520`, `xtask/src/package.rs:1551-1630` | Dummy-Datei `out/host/extra.txt` anlegen; `cargo xtask package --check` schlägt mit `unexpected package entry` fehl. Copyleft-Token in `host/THIRD_PARTY.md` führt zu `copyleft/MPL license terms found`. |
| **F5-C1:** Das Tray-Icon leitet jede Benutzergeste über den zentralen `Command`-Pfad der App. | **Bestätigt** | `crates/rustamp-platform/src/tray.rs:135-148`, `crates/rustamp-app/src/window.rs:1320-1326`, `window.rs:1385-1415` | Linksklick, Doppelklick und Kontextmenü-Auswahl rufen ausnahmslos `submit_platform_commands()` auf, welches `self.app.submit(command)` ausführt. Es existiert kein Nebenpfad. |

---

## 3. Befunde nach Schweregrad (Findings)

### Schweregrad: KRITISCH (Critical)

#### [Nachgewiesen] W16-F01: Abspiel-Divergenz und Desynchronisation zwischen `Engine` und `Worker` im Shuffle-Betrieb
- **Betroffene Komponenten:** `crates/rustamp-audio/src/engine.rs:633-650, 1523-1533`, `crates/rustamp-audio/src/decode.rs:704-716`
- **Typ:** Logikfehler / Architekturfehler
- **Beschreibung:**
  Wenn die Wiedergabe gestartet wird (`engine.play()` oder `play_index()`), ruft `Engine` die Methode `replan_shuffle_engine(None)` auf. Diese generiert `self.shuffle_plan` basierend auf `self.shuffle_seed`.
  Unmittelbar danach ruft `play()` die Methode `self.start_run()` auf. In Zeile 1528 wird der Seed mutiert:
  `self.shuffle_seed = self.shuffle_seed.wrapping_add(1);`
  Der Decode-Thread `Worker` wird mit diesem **inkrementierten** Seed gestartet (`Worker::spawn(..., self.shuffle_seed, ...)`).
  In `Worker::run(start_index)` generiert `Worker::init_shuffle_plan(start_index)` seinen eigenen Plan mit dem neuen Seed!
  **Auswirkung:** Die von `Engine::shuffle_plan()` an UI, Tests und Playlist gemeldete Reihenfolge stimmt **nicht** mit den tatsächlich vom Decoder abgespielten Titeln überein! Unit-Tests wie `tests/shuffle_repeat.rs:manual_advance_shuffle_history` testen lediglich die Zustände in `Engine`, bemerken jedoch nicht, dass der `Worker` völlig andere Songs decodiert.
- **Kleinste Reproduktion:**
  ```rust
  let mut engine = Engine::new(config_with_queue(vec![track0, track1, track2, track3]));
  engine.set_shuffle(true);
  engine.play().unwrap();
  let reported_plan = engine.shuffle_plan().to_vec();
  // Decode-Thread startet. Der erste von DecodeOut::TrackStarted gemeldete
  // Nachfolger nach track0 stimmt nicht mit reported_plan[1] überein!
  ```

#### [Nachgewiesen] W16-F02: `rustamp-playlist/src/order.rs` ist toter Code (Orphaned Implementation)
- **Betroffene Komponenten:** `crates/rustamp-playlist/src/order.rs`, `crates/rustamp-audio`, `crates/rustamp-app`
- **Typ:** Architektur-Bruch / Spezifikationsverstoß
- **Beschreibung:**
  Die Task Card `docs/tasks/S15-shuffle-repeat-impl.md` behauptet, die Shuffle- und Repeat-Logik über `crates/rustamp-playlist/src/order.rs` bereitgestellt zu haben. In der Realität ist `PlaybackOrder` vollständig verwaist. Weder `rustamp-audio` noch `rustamp-app` importieren `rustamp_playlist::order::PlaybackOrder` oder greifen darauf zu. Stattdessen wurden zwei weitere, voneinander abweichende Shuffle-Implementationen in `crates/rustamp-audio/src/engine.rs` und `crates/rustamp-audio/src/decode.rs` dupliziert.
- **Kleinste Reproduktion:**
  `git grep "PlaybackOrder" crates/` zeigt Verwendungen ausschließlich innerhalb von Unit-Tests in `rustamp-playlist`.

---

### Schweregrad: HOCH (High)

#### [Nachgewiesen] W16-F03: Shuffle-Zustand überlebt das Warteschlangenende nicht (Fehlstart bei Next im Stop-Zustand)
- **Betroffene Komponenten:** `crates/rustamp-audio/src/decode.rs:575-578`, `crates/rustamp-audio/src/engine.rs:966-978, 1367-1372`
- **Typ:** Zustandsfehler / Spezifikationsverletzung (S15 §2)
- **Beschreibung:**
  Wenn ein Shuffle-Durchlauf bei deaktiviertem Repeat (`repeat = false`) den letzten Titel der Permutation beendet hat, meldet der `Worker` `Outcome::Ended` und beendet sich. `Engine::finish_run()` setzt `self.transport = TransportState::Stopped`.
  Während des Abspielens wurde jedoch im `Engine`-Objekt die Variable `self.shuffle_pos` niemals hochgezählt (diese verblieb auf `0`, da nur der `Worker` intern `shuffle_pos` weiterschaltete).
  Wenn der Benutzer nun im gestoppten Zustand `engine.next_track()` drückt:
  ```rust
  // engine.rs:967
  if self.shuffle_pos + 1 < self.shuffle_plan.len() {
      self.shuffle_pos += 1;
      let next_index = self.shuffle_plan[self.shuffle_pos];
      return self.play_index(next_index);
  }
  ```
  Da `self.shuffle_pos == 0` ist, evaluiert `0 + 1 < len` zu `true`! Die Engine springt unerwartet an und beginnt, `shuffle_plan[1]` abzuspielen, anstatt im gestoppten Zustand zu verharren.
- **Kleinste Reproduktion:**
  1. Queue mit 3 Elementen, `shuffle = true`, `repeat = false`.
  2. Alle 3 Titel normal durchlaufen lassen, bis die Wiedergabe stoppt (`TransportState::Stopped`).
  3. `engine.next_track()` aufrufen.
  4. **Ergebnis:** Wiedergabe startet mit `shuffle_plan[1]`, statt gestoppt zu bleiben.

#### [Nachgewiesen] W16-F04: Verletzung von S15 §3.2 beim Zyklus-Sprung mit `Prev`
- **Betroffene Komponenten:** `crates/rustamp-audio/src/decode.rs:579-581, 604-607`
- **Typ:** Logikfehler / Spezifikationsverletzung
- **Beschreibung:**
  Gemäß S15 §3.2 muss bei `pos == 0` und aktivem `Repeat` ein Druck auf `Prev` an das Ende des **vorherigen** Zyklus springen.
  In `decode.rs:579` wird bei Erreichen des Zyklusendes `replan_shuffle_new_cycle()` aufgerufen. Dies überschreibt `self.shuffle_plan` sofort mit der Permutation des neuen Zyklus $k+1$ und setzt `self.shuffle_pos = 0`.
  Wird nun auf dem ersten Track des neuen Zyklus `Prev` gedrückt, führt Zeile 604 folgendes aus:
  `self.shuffle_pos = self.shuffle_plan.len() - 1;`
  Dies springt an das Ende von Zyklus $k+1$ (Titel, die im aktuellen Zyklus noch gar nicht gespielt wurden), anstatt zum letzten Titel von Zyklus $k$. Die Historie des vorherigen Zyklus wird komplett verworfen.
- **Kleinste Reproduktion:**
  1. Queue `[0, 1, 2]`, `shuffle = true, repeat = true`.
  2. Zyklus 1 vollständig abspielen lassen.
  3. Sobald der erste Track von Zyklus 2 startet, `prev_track()` aufrufen.
  4. **Ergebnis:** Es wird `shuffle_plan_cycle2[2]` abgespielt, statt `shuffle_plan_cycle1[2]`.

#### [Nachgewiesen] W16-F05: Verletzung der Anti-Wiederholungs-Garantie bei Mid-Run Shuffle Toggle
- **Betroffene Komponenten:** `crates/rustamp-audio/src/decode.rs:722-724, 698-705`
- **Typ:** Logikfehler / Spezifikationsverletzung (S15 §4)
- **Beschreibung:**
  Wenn Shuffle während der Wiedergabe ein- oder umgeschaltet wird, ruft `decode.rs` die Funktion `replan_shuffle_mid_run(current_index)` auf. Diese delegiert direkt an `self.init_shuffle_plan(current)`.
  In `init_shuffle_plan` wird der Pool aller verbleibenden Tracks jedoch wie folgt gebildet:
  `let mut remaining: Vec<usize> = (0..len).filter(|&i| i != start_index).collect();`
  Dadurch werden alle Titel der Queue, die vor dem Umschalten im selben Durchlauf bereits erklungen sind, erneut in den Shuffle-Pool aufgenommen. S15 §4 ("Toggle mid-run") verbietet die Wiederholung bereits gespielter Tracks vor Beendigung des Zyklus.
- **Kleinste Reproduktion:**
  1. Queue `[0, 1, 2, 3]`.
  2. Titel 0 abspielen, manuell zu Titel 1 springen.
  3. Shuffle aktivieren.
  4. **Ergebnis:** Titel 0 befindet sich wieder im Plan und kann unmittelbar als nächster Track nach Titel 1 gewählt werden.

---

### Schweregrad: MITTEL (Medium)

#### [Nachgewiesen] W16-F06: Visualisierer löscht den Bildschirm nach Stop im Spektrum-Modus nicht sofort
- **Betroffene Komponenten:** `crates/rustamp-ui/src/visualization.rs:264-294, 388-410`, `crates/rustamp-app/src/visualization_tests.rs:210-226`
- **Typ:** Spezifikations- und Test-Diskrepanz
- **Beschreibung:**
  Die Aufgabenkarte behauptet, dass der Visualisierer den Bildschirm nach einem Stop sofort bereinigt ("really clear the screen after a stop").
  In `crates/rustamp-ui/src/visualization.rs` bewirkt `TransportState::Stopped` jedoch lediglich das Nullen von `targets` und das Setzen von `state.input = VisInput::Missing`.
  - Im Oszilloskop-Modus (`VisMode::Scope`) wird die Welle durch die Bedingung `state.input == VisInput::Fresh` tatsächlich sofort ausgeblendet.
  - Im Spektrum-Modus (`VisMode::Spectrum`) prüft `draw_visualization` `state.input` **nicht**. Die Balken fallen mit 24 Zeilen/Sekunde und die Peaks nach 300 ms Hold-Zeit mit 16 Zeilen/Sekunde. Bis zu **1.300 ms** nach dem Stop werden weiterhin Spektrum-Balken und Peak-Marker auf die Bildschirmoberfläche gezeichnet!
  In `crates/rustamp-app/src/visualization_tests.rs:213` wurde dieser Umstand umgangen, indem der Test vor dem Stop explizit auf `VisMode::Scope` umschaltet (`app.ui.set_vis_mode(VisMode::Scope)`), um den Test `assert_static_pixels` künstlich grün zu halten.
- **Kleinste Reproduktion:**
  ```rust
  let (mut app, _dir) = playing();
  app.ui.set_vis_mode(VisMode::Spectrum);
  app.submit(Command::Stop).unwrap();
  // Unmittelbar nach Stop prüfen:
  assert_static_pixels(&mut app); // SCHLÄGT FEHL: Spektrum-Balken noch gerendert!
  ```

#### [Nachgewiesen] W16-F07: Stille-Substitution für nicht-finite PCM-Werte vor dem Visualisierer
- **Betroffene Komponenten:** `crates/rustamp-audio/src/decode.rs:928-932`, `crates/rustamp-ui/src/visualization.rs:136-138`
- **Typ:** Widersprüchliche Fehlerbehandlung / Stille Datenkorrektur
- **Beschreibung:**
  `rustamp-ui` implementiert die S08-Vorgabe: "Invalid/non-finite PCM or rate 0 returns an error, not substituted audio. Missing input never becomes a synthetic zero waveform."
  In `crates/rustamp-audio/src/decode.rs:928` überschreibt der Decoder jedoch fehlerhafte Samples vor dem Einspeisen in `VisTap`:
  ```rust
  for sample in block.iter_mut() {
      if !sample.is_finite() {
          *sample = 0.0;
      }
  }
  self.vis.push(block);
  ```
  Durch diese Stille-Substitution fabriziert die Audio-Engine synthetische Null-Samples, und der definierte Fehler `VisError::NonFinitePcm` kann im laufenden Betrieb niemals ausgelöst werden.
- **Kleinste Reproduktion:**
  Einen Decoder-Block mit `f32::NAN` einspeisen. Im Visualisierer kommt eine glatte Nulllinie (`0.0`) an, anstatt dass der Frame mit `VisError::NonFinitePcm` abgewiesen wird.

#### [Nachgewiesen] W16-F08: Lizenzinventar unterschlägt Volltext der MPL-2.0 und Apache-2.0
- **Betroffene Komponenten:** `xtask/src/package.rs:1172, 1220-1234, 1251-1262`
- **Typ:** Dokumentations- / Compliance-Mangel
- **Beschreibung:**
  Die Task Card X10 und der Doc-Kommentar in `xtask/src/package.rs:1172` behaupten explizit:
  *"4. Symphonia MPL-2.0 notice, source availability, and full license text."*
  In `player_third_party_notices()` wird jedoch in Abschnitt 4 lediglich eine 14-zeilige Notiz mit Link auf `https://www.mozilla.org/en-US/MPL/2.0/` ausgegeben. Der vollständige Lizenztext der Mozilla Public License v2.0 fehlt im erzeugten `THIRD_PARTY.md`.
  Ebenso enthält Abschnitt 5 für die `Apache License, Version 2.0` nur den standardmäßigen 11-zeiligen Quellcode-Header ("Licensed under the Apache License, Version 2.0..."), nicht aber die eigentlichen Lizenzbedingungen (Terms and Conditions).

---

### Schweregrad: NIEDRIG (Low)

#### [Nachgewiesen] W16-F09: Veralteter Hinweis (Placebo Disclaimer) in generierter Release-`README.md`
- **Betroffene Komponenten:** `xtask/src/package.rs:1133`
- **Typ:** Veraltete Dokumentation
- **Beschreibung:**
  In `xtask/src/package.rs` enthält das Template für die im Release-ZIP enthaltene `README.md` unter Abschnitt 4 ("Unproven / Not Implemented") folgenden veralteten Satz:
  *"Visualizer & Placebos: Spectrum analyzer is contracted to zero. Shuffle and repeat UI toggles are not yet connected to the playback engine."*
  Obwohl in dieser Wave sowohl der Visualisierer als auch die Shuffle/Repeat-Anbindung implementiert wurden, wird dem Endanwender im Release-Paket mitgeteilt, dass diese Funktionen Placebos seien.

---

## 4. Was ohne VM / realen Desktop nicht verifiziert werden kann

Folgende Aspekte der untersuchten Komponenten stützen sich auf Win32-Subsysteme, Desktop-Compositor-Eigenschaften oder reale Hardware und können im reinen CI/Headless-Workspace ohne VM oder realen Desktop **nicht** abschließend verifiziert werden:

1. **Win32 Shell Notification Area (System-Tray) Interaktion:**
   - Ob das aus ASCII-Art gerenderte 16x16 / High-DPI BGRA-Icon (`crates/rustamp-platform/src/tray.rs:scaled_bgra`) von Windows 10/11 tatsächlich ohne Darstellungsfehler oder Farbsäume im Notification-Area platziert wird (`Shell_NotifyIconW(NIM_ADD)`).
   - Ob das Subclassing (`SetWindowSubclass`) auf dem `HWND` des Fensters die Nachrichtenfolge `WM_LBUTTONUP` und `WM_LBUTTONDBLCLK` unter allen Windows-Versionen zuverlässig zu genau einem Fenster-Toggle zusammenfasst.
   - Ob `TrackPopupMenu` bei Verlust des Eingabefokus (z. B. Klick auf eine andere Anwendung) durch den Win32-Workaround `SetForegroundWindow` + `PostMessageW(WM_NULL)` das Menü immer sauber schließt, ohne hängende Menüschatten oder UI-Blockaden zu hinterlassen.
   - Ob der Empfang der Broadcast-Nachricht `RegisterWindowMessageW("TaskbarCreated")` bei einem echten Absturz und Neustart von `explorer.exe` tatsächlich zur nahtlosen Wiederherstellung des Tray-Icons führt.

2. **Reale Audio-Hardware-Latenz vs. Visualisierer-Synchronität:**
   - Der `VisTap` greift PCM-Puffer auf dem Decoder-Thread ab (`crates/rustamp-audio/src/vistap.rs`). Da der Decoder der tatsächlichen Hardware-Wiedergabe (WASAPI-Endpoint-Puffer) um mehrere Blöcke vorauseilt, kann die reale Synchronität zwischen sichtbarem Ausschlag auf dem Monitor und hörbarem Ton über die Soundkarte ohne physisches Audio-Interface und Bildschirmmessung nicht validiert werden.

3. **PE-Header, CRT-Statik und Clean-OS-Start:**
   - Obwohl `xtask/src/package.rs` mit `-C target-feature=+crt-static` baut, bleibt das Feld `"artifact_imports_verified": false` im Manifest bestehen. Ob die erzeugten Binärdateien (`rustamp.exe`, `rustamp-plughost32.exe`) auf einer absolut frischen Windows-Installation ohne installierte Visual C++ Redistributable-Laufzeitumgebungen und ohne Entwicklertools starten, kann nur in einer sauberen VM geprüft werden.
