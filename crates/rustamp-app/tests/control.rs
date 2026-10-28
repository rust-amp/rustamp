// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The local control channel (`--control`), end to end and **headless**.
//!
//! The real `rustamp.exe` is spawned with `--null --seconds --control=<name>`
//! and driven through [`rustamp_testkit::control::ControlClient`]: the requests
//! and the answers are **typed** (`rustamp-core`'s `Command`/`Event`,
//! `PlayerState`), so this file replaces the string-greps the headless tests
//! use with assertions on the player's own protocol.
//!
//! # GUI rule: no spawned child may raise a window
//!
//! `--null` opens no window, and every spawn goes through [`binary`], which
//! sets `CREATE_NO_WINDOW` — a console window counts as a window on the host.
//! The child also carries `RUSTAMP_NO_GUI=1` ([`isolate`]), so a run that lost
//! its `--null` fails instead of opening the app's window.
//!
//! # Why a name of its own
//!
//! `--control=<name>` names the pipe after a key this test owns
//! ([`unique_name`]), so the test never reaches — and never collides with — a
//! player the developer is running. The player's own default is the
//! single-instance key, which two test runs would share.

use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::time::{Duration, Instant};

use rustamp_core::command::Command;
use rustamp_core::event::Event;
use rustamp_core::playlist::{MediaLocation, PlaylistChangeKind};
use rustamp_core::state::TransportState;
use rustamp_testkit::control::{ControlClient, EventFilter};

/// `CreateProcess` flag: run the child **without a console window**
/// (`CREATE_NO_WINDOW`).
///
/// The same helper as `tests/headless.rs`: without it a console-subsystem
/// child gets a console window of its own when the parent has no console, and
/// a console window counts as a window on the host.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Makes `command` incapable of raising a window.
fn no_window(command: &mut Process) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
        CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    {
        let _ = command;
        0
    }
}

/// The `rustamp` binary under test, spawned without a window.
fn binary() -> Process {
    let mut command = Process::new(env!("CARGO_BIN_EXE_rustamp"));
    no_window(&mut command);
    command
}

/// An isolated configuration, the GUI rail and the no-window flag.
fn isolate(command: &mut Process, dir: &tempfile::TempDir) {
    no_window(command);
    command.env("RUSTAMP_CONFIG", dir.path().join("rustamp.toml"));
    command.env(rustamp_app::window::NO_GUI_ENV, "1");
}

/// The repo's own 1.5 s, 440 Hz FLAC fixture.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join("sine.flac")
}

/// A channel name that no other test in this binary (or player) uses.
///
/// One test binary runs its tests in parallel, so the process id alone is not
/// unique: the label of the test and a counter are part of the name.
fn unique_name(label: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_millis())
        .unwrap_or(0);
    format!("test-{}-{n}-{label}-{millis}", std::process::id())
}

/// Kills the child when the test ends, and prints what the player said if the
/// test is failing (the child's output is not on the test's own streams).
struct Player {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Player {
    /// Starts the player with `--null`, the control channel and no media.
    ///
    /// `--seconds` bounds the run: nothing plays until the test sends a
    /// command, so the limit is the test's backstop against a player that
    /// never stops.
    fn start(dir: &tempfile::TempDir, name: &str) -> Self {
        let stdout = dir.path().join("player.stdout");
        let stderr = dir.path().join("player.stderr");
        let mut command = binary();
        isolate(&mut command, dir);
        let child = command
            .arg("--null")
            .arg("--seconds")
            .arg("30")
            .arg(format!("--control={name}"))
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn rustamp");
        Player {
            child,
            stdout,
            stderr,
        }
    }

    /// What the player wrote to the standard output so far.
    fn output(&self) -> String {
        std::fs::read_to_string(&self.stdout).unwrap_or_default()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            eprintln!(
                "player stdout:\n{}\nplayer stderr:\n{}",
                self.output(),
                std::fs::read_to_string(&self.stderr).unwrap_or_default()
            );
        }
    }
}

/// Copies the fixture under `name`, so the playlist has rows with known titles.
fn copy_fixture(dir: &tempfile::TempDir, name: &str) -> MediaLocation {
    let path = dir.path().join(name);
    std::fs::copy(fixture(), &path).expect("the fixture is copied");
    MediaLocation::File(path)
}

/// Plays queue row `index` and returns the app's stable entry id of that row.
///
/// The id is the one the app assigns, not one the engine knows: `PlayIndex`
/// makes the row current, and the snapshot reports the current entry's id.
/// `TrackChanged` says the track opened, so the probe reads the snapshot only
/// once the player really switched.
fn entry_id_at(client: &mut ControlClient, index: u32) -> u64 {
    let response = client
        .send(Command::PlayIndex(index))
        .expect("the player answers PlayIndex");
    assert!(response.ok, "PlayIndex was refused: {}", response.message);
    client
        .wait_for(
            |event| matches!(event, Event::TrackChanged { index: Some(row), .. } if *row == index),
            Duration::from_secs(10),
        )
        .expect("the player opened the row it was asked to play");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = client.snapshot().expect("the player answers a snapshot");
        if state.current_index == Some(index) {
            return state
                .current_id
                .unwrap_or_else(|| panic!("row {index} has no stable entry id"))
                .0;
        }
        assert!(
            Instant::now() < deadline,
            "the snapshot never reported row {index} as current: {state:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The ids of the rows `0..count`, in playlist order.
fn ids_in_order(client: &mut ControlClient, count: u32) -> Vec<u64> {
    (0..count).map(|index| entry_id_at(client, index)).collect()
}

#[test]
fn the_control_channel_drives_and_inspects_the_running_player() {
    let dir = tempfile::tempdir().unwrap();
    let name = unique_name("drive");
    let player = Player::start(&dir, &name);
    let pipe = rustamp_app::control::pipe_name(&name);
    let mut client = ControlClient::connect(&pipe)
        .unwrap_or_else(|error| panic!("{pipe} (player output: {}): {error}", player.output()));

    // ----- stage 1: ping and snapshot, no command involved -----
    let pong = client.ping().expect("the player answers a ping");
    assert!(pong.ok, "{}", pong.message);
    assert_eq!(pong.message, "pong");
    assert!(
        pong.state.is_some(),
        "a ping answer carries the snapshot as well"
    );
    let state = client.snapshot().expect("the player answers a snapshot");
    assert_eq!(state.playlist_len, 0, "nothing is queued yet: {state:?}");
    assert_eq!(state.transport, TransportState::Stopped);
    assert_eq!(state.current_index, None);

    // ----- stage 2: AddItems, and the event that confirms it -----
    // The add order is b, a, c, so a sort by title must produce a, b, c.
    let b = copy_fixture(&dir, "b.flac");
    let a = copy_fixture(&dir, "a.flac");
    let c = copy_fixture(&dir, "c.flac");
    let response = client
        .send(Command::AddItems {
            items: vec![b, a, c],
            at: None,
            play: false,
        })
        .expect("the player answers AddItems");
    assert!(response.ok, "{}", response.message);
    let event = client
        .wait(EventFilter::PlaylistChanged, Duration::from_secs(10))
        .expect("the player answers the typed wait");
    assert!(event.ok, "{}", event.message);
    assert!(
        matches!(
            event.event(),
            Some(Event::PlaylistChanged {
                kind: PlaylistChangeKind::Inserted,
                len: 3
            })
        ),
        "AddItems publishes Inserted with the new length: {:?}",
        event.event()
    );
    assert_eq!(
        client.snapshot().unwrap().playlist_len,
        3,
        "the snapshot agrees with the event"
    );

    // ----- the add order, measured rather than assumed -----
    let added_order = ids_in_order(&mut client, 3);
    let mut distinct = added_order.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        3,
        "every row has its own id: {added_order:?}"
    );

    // ----- Sort(Title) reorders the rows, and the ids say how -----
    let response = client
        .send(Command::Sort(rustamp_core::playlist::SortKey::Title))
        .expect("the player answers Sort");
    assert!(response.ok, "{}", response.message);
    let event = client
        .wait(EventFilter::PlaylistChanged, Duration::from_secs(10))
        .expect("the player answers the typed wait");
    assert!(event.ok, "{}", event.message);
    assert!(
        matches!(
            event.event(),
            Some(Event::PlaylistChanged {
                kind: PlaylistChangeKind::Reordered,
                len: 3
            })
        ),
        "Sort publishes Reordered with the new length: {:?}",
        event.event()
    );
    let sorted_order = ids_in_order(&mut client, 3);
    assert_eq!(
        sorted_order,
        vec![added_order[1], added_order[0], added_order[2]],
        "titles sort a, b, c from the add order b, a, c"
    );

    // ----- Play, and the transport event it confirms -----
    let response = client.send(Command::Stop).expect("the player answers Stop");
    assert!(response.ok, "{}", response.message);
    client
        .wait_for(
            |event| matches!(event, Event::StateChanged(TransportState::Stopped)),
            Duration::from_secs(10),
        )
        .expect("Stop stops the transport");
    let response = client.send(Command::Play).expect("the player answers Play");
    assert!(response.ok, "{}", response.message);
    client
        .wait_for(
            |event| matches!(event, Event::StateChanged(TransportState::Playing)),
            Duration::from_secs(10),
        )
        .expect("Play starts the transport");

    // ----- Seek, and the position event it confirms -----
    let response = client
        .send(Command::Seek { to_ms: 700 })
        .expect("the player answers Seek");
    assert!(response.ok, "{}", response.message);
    let event = client
        .wait_for(
            |event| matches!(event, Event::PositionChanged { position_ms, .. } if *position_ms >= 700),
            Duration::from_secs(10),
        )
        .expect("Seek moves the position");
    assert!(
        matches!(event, Event::PositionChanged { position_ms, .. } if position_ms >= 700),
        "the seek landed at {event:?}"
    );

    // ----- the run itself announces the channel -----
    assert!(
        player.output().contains("control channel"),
        "the player announces the channel it serves: {}",
        player.output()
    );
}

/// The gate: the control channel is refused together with the window it would
/// need.
///
/// A windowed run under the GUI rail fails before the pipe exists, so a client
/// cannot reach a player the rail has refused. This is headless (the run
/// refuses at once, nothing is opened).
#[test]
fn the_control_channel_is_gated_by_the_window_rail() {
    let dir = tempfile::tempdir().unwrap();
    let name = unique_name("gate");
    let mut command = binary();
    isolate(&mut command, &dir); // sets RUSTAMP_NO_GUI=1
    let output = command
        .arg("--no-audio") // the silent windowed mode: a window, no device
        .arg(format!("--control={name}"))
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a gated windowed run must fail\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("test VM"),
        "the rail's own message must be the refusal; stderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("control channel"),
        "a refused run must not serve a channel; stdout:\n{stdout}"
    );
    let pipe = rustamp_app::control::pipe_name(&name);
    assert!(
        ControlClient::connect_within(&pipe, Duration::from_millis(300)).is_err(),
        "{pipe} must not accept a connection after the rail refused the run"
    );
}

// ---- spawn-site guard: the source check below ignores everything after this
// line, so its own needles cannot match themselves. ----

/// The split marker for [`every_spawn_site_is_guarded`].
const SPAWN_GUARD_MARKER: &str = "// ---- spawn-site guard";

/// Guards the spawn sites of this file: exactly one place constructs a child,
/// and it applies `CREATE_NO_WINDOW` (see `tests/headless.rs`, which guards
/// its own file the same way).
#[test]
fn every_spawn_site_is_guarded() {
    let source = include_str!("control.rs");
    let production = source.split(SPAWN_GUARD_MARKER).next().unwrap_or(source);
    assert_eq!(
        production.matches("Process::new(").count(),
        1,
        "there must be exactly one spawn site (`binary()`); route new spawns through it"
    );
    assert!(
        production.contains("creation_flags(CREATE_NO_WINDOW)"),
        "`no_window` must set `CREATE_NO_WINDOW`"
    );
    assert!(
        production.contains("no_window(&mut command)"),
        "`binary()` must apply the no-window flag"
    );
}

/// The pipe name the test connects to is the one `--control=<name>` asks for.
#[test]
fn the_name_reaches_the_pipe() {
    let pipe = rustamp_app::control::pipe_name("abc");
    assert_eq!(pipe, r"\\.\pipe\rustamp-ctl-abc");
    assert!(
        Path::new(&pipe).is_absolute() || pipe.starts_with(r"\\.\pipe\"),
        "the channel is in the local pipe namespace: {pipe}"
    );
}
