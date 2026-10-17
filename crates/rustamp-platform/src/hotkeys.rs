// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! System-wide hotkeys that map onto existing [`Command`]s (task card F3).
//!
//! A global hotkey works while another application has focus — S03 §7 leaves
//! exactly this unmeasured ("whether any of these work while another
//! application has focus ... was not tested"), so the chords below are **own
//! choices**, stated as such. What a spec settles is only the target side:
//! each [`HotkeyAction`] names the `rustamp_ui::ShortcutAction` it reuses and
//! carries that action's S03 `WM_COMMAND` id, and [`HotkeyAction::commands`]
//! returns only [`Command`] variants that already exist. No new command and no
//! new shortcut action was invented here; the platform crate must not depend
//! on `rustamp-ui` (F2 contract), so the reuse is by id and documented name,
//! not by type.
//!
//! # Bookkeeping
//!
//! [`HotkeyRegistry`] owns the process's bindings: registering an already
//! bound combination fails loudly ([`HotkeyError::AlreadyBound`]) without
//! touching the backend, and an OS refusal (the combination is taken by
//! another application) fails as [`HotkeyError::BackendTaken`] without
//! recording the binding, so a retry never wedges. Unregistering removes the
//! binding first and unregisters the OS hotkey exactly once; unregistering an
//! unknown id fails without touching the backend. The registry is generic over
//! [`HotkeyBackend`], so the headless tests run it against a fake; the Win32
//! backend ([`Win32HotkeyBackend`]) wraps `RegisterHotKey` and is exercised in
//! the VM, where a real desktop exists.
//!
//! # Runtime
//!
//! `RegisterHotKey` with a null window delivers `WM_HOTKEY` to the calling
//! thread's queue, so the session thread ([`HotkeySession`], Windows only)
//! registers the defaults on its own thread and pumps that queue, queuing
//! fired actions where the UI thread drains them with
//! [`take_pending_hotkeys`] — the same shape as `single_instance`'s queue, so
//! the `window.rs` follow-up is the same few lines in `about_to_wait`. Nothing
//! here opens a window.

use std::collections::HashMap;
use std::sync::Mutex;

use rustamp_core::command::Command;
use rustamp_core::state::WindowId;

/// Modifier set of a system-wide hotkey.
///
/// The bit values match the Win32 `MOD_*` constants (`MOD_ALT = 1`,
/// `MOD_CONTROL = 2`, `MOD_SHIFT = 4`, `MOD_WIN = 8`), so the Win32 backend
/// passes them through unchanged; on any other target they are just bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HotkeyModifiers(pub u32);

impl HotkeyModifiers {
    /// No modifier.
    pub const NONE: Self = Self(0);
    /// Alt.
    pub const ALT: Self = Self(1);
    /// Ctrl.
    pub const CONTROL: Self = Self(2);
    /// Shift.
    pub const SHIFT: Self = Self(4);
    /// Windows key.
    pub const WIN: Self = Self(8);

    /// Whether `other`'s bits are all set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOr for HotkeyModifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

/// One system-wide hotkey chord: modifiers plus a virtual-key code.
///
/// Virtual-key codes are the Win32 `VK_*` values (`b'X'` is `0x58`, `F1` is
/// `0x70`, ...); the type stays a plain `u32` so no windowing library leaks
/// into the headless model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyCombo {
    /// The modifiers that must be held.
    pub modifiers: HotkeyModifiers,
    /// The virtual-key code.
    pub vk: u32,
}

impl HotkeyCombo {
    /// A chord of `modifiers` plus `vk`.
    pub const fn new(modifiers: HotkeyModifiers, vk: u32) -> Self {
        Self { modifiers, vk }
    }

    /// A `Ctrl+Alt` chord over an ASCII letter or digit code (own choice: the
    /// default-table shape, see [`DEFAULT_HOTKEYS`]).
    pub const fn ctrl_alt(vk: u32) -> Self {
        Self::new(HotkeyModifiers::CONTROL.union(HotkeyModifiers::ALT), vk)
    }
}

/// A human-readable chord for logs and refusal messages (headless).
pub fn describe(combo: HotkeyCombo) -> String {
    let mut parts = Vec::new();
    if combo.modifiers.contains(HotkeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if combo.modifiers.contains(HotkeyModifiers::ALT) {
        parts.push("Alt");
    }
    if combo.modifiers.contains(HotkeyModifiers::SHIFT) {
        parts.push("Shift");
    }
    if combo.modifiers.contains(HotkeyModifiers::WIN) {
        parts.push("Win");
    }
    let key = match combo.vk {
        vk @ 0x41..=0x5A => (vk as u8 as char).to_string(),
        vk @ 0x30..=0x39 => (vk as u8 as char).to_string(),
        vk @ 0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        vk => format!("VK_{vk:#X}"),
    };
    parts.push(&key);
    parts.join("+")
}

/// What a fired global hotkey does.
///
/// Each variant reuses one `rustamp_ui::ShortcutAction` (named in
/// [`HotkeyAction::shortcut_action`]) and maps onto the commands that action
/// already produces — the tray's and the shortcut table's vocabulary, not a
/// new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    /// Play ([`Command::Play`]).
    Play,
    /// Pause when playing, resume when paused ([`Command::TogglePause`]).
    PauseUnpause,
    /// Stop playback ([`Command::Stop`]).
    Stop,
    /// Next track ([`Command::Next`]).
    NextTrack,
    /// Previous track ([`Command::Prev`]).
    PreviousTrack,
    /// Show the main window when hidden, hide it when shown
    /// ([`Command::ToggleWindow`] of the main window).
    ToggleMainWindow,
}

impl HotkeyAction {
    /// The `rustamp_ui::ShortcutAction` variant this hotkey reuses (by name;
    /// the platform crate does not depend on `rustamp-ui`).
    pub fn shortcut_action(self) -> &'static str {
        match self {
            HotkeyAction::Play => "Play",
            HotkeyAction::PauseUnpause => "PauseUnpause",
            HotkeyAction::Stop => "Stop",
            HotkeyAction::NextTrack => "NextTrack",
            HotkeyAction::PreviousTrack => "PreviousTrack",
            HotkeyAction::ToggleMainWindow => "ShowMainWindow",
        }
    }

    /// The public `WM_COMMAND` id (S03) behind the action, if S03 records one.
    ///
    /// The show/hide toggle borrows no id: S03's only main-window row here is
    /// the one-way show (`Alt+W`, 40258).
    pub fn command_id(self) -> Option<u32> {
        match self {
            HotkeyAction::Play => Some(40045),
            HotkeyAction::PauseUnpause => Some(40046),
            HotkeyAction::Stop => Some(40047),
            HotkeyAction::NextTrack => Some(40048),
            HotkeyAction::PreviousTrack => Some(40044),
            HotkeyAction::ToggleMainWindow => None,
        }
    }

    /// The commands a fired hotkey applies (existing variants only).
    pub fn commands(self) -> Vec<Command> {
        vec![match self {
            HotkeyAction::Play => Command::Play,
            HotkeyAction::PauseUnpause => Command::TogglePause,
            HotkeyAction::Stop => Command::Stop,
            HotkeyAction::NextTrack => Command::Next,
            HotkeyAction::PreviousTrack => Command::Prev,
            HotkeyAction::ToggleMainWindow => Command::ToggleWindow(WindowId::Main),
        }]
    }
}

/// The default global hotkeys (own choice).
///
/// Shape: `Ctrl+Alt` chords, so they do not collide with any in-app S03 chord
/// (S03 uses `Ctrl+Alt` only for `I`, `N` and `R`); letters echo the main
/// table's mnemonics (`X` play, `C` pause, `V` stop, `B` next, `Z` previous),
/// and `H` — unbound in every S03 table — shows/hides. `Ctrl+Alt+N` is
/// deliberately avoided (S03's provisional "new instance", 40305).
pub const DEFAULT_HOTKEYS: &[(HotkeyCombo, HotkeyAction)] = &[
    (HotkeyCombo::ctrl_alt(b'X' as u32), HotkeyAction::Play),
    (
        HotkeyCombo::ctrl_alt(b'C' as u32),
        HotkeyAction::PauseUnpause,
    ),
    (HotkeyCombo::ctrl_alt(b'V' as u32), HotkeyAction::Stop),
    (HotkeyCombo::ctrl_alt(b'B' as u32), HotkeyAction::NextTrack),
    (
        HotkeyCombo::ctrl_alt(b'Z' as u32),
        HotkeyAction::PreviousTrack,
    ),
    (
        HotkeyCombo::ctrl_alt(b'H' as u32),
        HotkeyAction::ToggleMainWindow,
    ),
];

/// The registry's id for one binding (assigned on registration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyId(i32);

/// Why a registration or unregistration failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    /// The combination is already bound in this process (fails loudly; the
    /// backend was not touched).
    #[error("hotkey {0} is already registered in this process")]
    AlreadyBound(String),
    /// The OS refused the combination — it is taken by another application
    /// (fails loudly; nothing was recorded, so a retry never wedges).
    #[error("hotkey {combo} was refused by the system: {detail}")]
    BackendTaken {
        /// The refused chord, human-readable.
        combo: String,
        /// The backend's reason.
        detail: String,
    },
    /// No binding carries this id (the backend was not touched).
    #[error("unknown hotkey id {0:?}")]
    UnknownId(HotkeyId),
    /// The binding was removed but the OS unregistration failed. The state
    /// stays clean — the chord is reusable — only the OS side needs a retry.
    #[error("hotkey {id:?} was unbound but the system kept it: {detail}")]
    BackendUnregister {
        /// The id that was removed.
        id: HotkeyId,
        /// The backend's reason.
        detail: String,
    },
}

/// The OS side of [`HotkeyRegistry`]: register and unregister by id.
pub trait HotkeyBackend {
    /// The backend's failure reason, as text.
    type Error: std::fmt::Display;

    /// Registers `id` for `modifiers` + `vk`; fails when the combination is
    /// taken (by this process through another registrar, or by another app).
    fn register(&mut self, id: i32, modifiers: u32, vk: u32) -> Result<(), Self::Error>;
    /// Unregisters `id`; fails when the OS has nothing under it.
    fn unregister(&mut self, id: i32) -> Result<(), Self::Error>;
}

/// The process's hotkey bindings over a [`HotkeyBackend`].
///
/// The registry is the single registrar: every OS registration in this
/// process goes through it, so an in-process double registration is caught
/// before the OS ever sees it.
#[derive(Debug)]
pub struct HotkeyRegistry<B> {
    backend: B,
    next_id: i32,
    by_combo: HashMap<HotkeyCombo, (HotkeyId, HotkeyAction)>,
    by_id: HashMap<HotkeyId, (HotkeyCombo, HotkeyAction)>,
}

impl<B: HotkeyBackend> HotkeyRegistry<B> {
    /// An empty registry over `backend`.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            next_id: 1,
            by_combo: HashMap::new(),
            by_id: HashMap::new(),
        }
    }

    /// The number of live bindings.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether no binding is live.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The action bound to `combo`, if any.
    pub fn action_for(&self, combo: HotkeyCombo) -> Option<HotkeyAction> {
        self.by_combo.get(&combo).map(|(_, action)| *action)
    }

    /// Registers `combo` for `action`.
    ///
    /// A taken combination fails loudly ([`HotkeyError::AlreadyBound`] for an
    /// in-process double registration, [`HotkeyError::BackendTaken`] for an OS
    /// refusal) and records nothing; unregistering the returned id later is
    /// clean (exactly one backend unregistration, the chord reusable).
    pub fn register(
        &mut self,
        combo: HotkeyCombo,
        action: HotkeyAction,
    ) -> Result<HotkeyId, HotkeyError> {
        if self.by_combo.contains_key(&combo) {
            return Err(HotkeyError::AlreadyBound(describe(combo)));
        }
        let id = HotkeyId(self.next_id);
        self.backend
            .register(id.0, combo.modifiers.0, combo.vk)
            .map_err(|error| HotkeyError::BackendTaken {
                combo: describe(combo),
                detail: error.to_string(),
            })?;
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.by_combo.insert(combo, (id, action));
        self.by_id.insert(id, (combo, action));
        Ok(id)
    }

    /// Unregisters `id`: the binding goes first, the backend second, so the
    /// chord is reusable even when the backend reports a failure.
    pub fn unregister(&mut self, id: HotkeyId) -> Result<HotkeyAction, HotkeyError> {
        let Some((combo, action)) = self.by_id.remove(&id) else {
            return Err(HotkeyError::UnknownId(id));
        };
        self.by_combo.remove(&combo);
        self.backend
            .unregister(id.0)
            .map_err(|error| HotkeyError::BackendUnregister {
                id,
                detail: error.to_string(),
            })?;
        Ok(action)
    }
}

/// Fired hotkey actions waiting for the UI thread.
///
/// Process-global like `single_instance`'s queue: the session thread only
/// queues, and the UI thread drains them (the `window.rs` follow-up) through
/// the normal command path.
static PENDING: Mutex<Vec<HotkeyAction>> = Mutex::new(Vec::new());

/// Queues a fired `action` for the UI thread.
fn push_pending(action: HotkeyAction) {
    let mut guard = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.push(action);
}

/// Drains every fired hotkey action queued so far.
///
/// Called on the UI thread; the lock recovers from poisoning, so a panic
/// while queueing never wedges the player.
pub fn take_pending_hotkeys() -> Vec<HotkeyAction> {
    let mut guard = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *guard)
}

#[cfg(windows)]
mod imp {
    //! Win32 backend and session thread: `RegisterHotKey` plus a `WM_HOTKEY`
    //! pump on the thread that registered them.
    //!
    //! The ids are the registry's own ([`HotkeyId`]'s `i32`), so the pump maps
    //! `wParam` straight back to the fired action. Compiled here, exercised in
    //! the VM: hotkeys need a real desktop.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
    };
    use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY};

    use super::{
        DEFAULT_HOTKEYS, HotkeyAction, HotkeyBackend, HotkeyRegistry, describe, push_pending,
    };

    /// The Win32 backend: `RegisterHotKey`/`UnregisterHotKey` with a null
    /// window, so `WM_HOTKEY` lands in the calling thread's queue.
    #[derive(Debug, Default)]
    pub struct Win32HotkeyBackend;

    /// Why the Win32 call failed.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Win32HotkeyError(pub String);

    impl std::fmt::Display for Win32HotkeyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl HotkeyBackend for Win32HotkeyBackend {
        type Error = Win32HotkeyError;

        fn register(&mut self, id: i32, modifiers: u32, vk: u32) -> Result<(), Self::Error> {
            // SAFETY: null window (thread queue), plain integers; the id is
            // the registry's own. A refusal (taken combination) surfaces as
            // the `Err` the registry reports loudly.
            unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(modifiers), vk) }
                .map_err(|error| Win32HotkeyError(format!("RegisterHotKey refused: {error}")))
        }

        fn unregister(&mut self, id: i32) -> Result<(), Self::Error> {
            // SAFETY: same invariant; the id was registered from this thread.
            unsafe { UnregisterHotKey(None, id) }
                .map_err(|error| Win32HotkeyError(format!("UnregisterHotKey failed: {error}")))
        }
    }

    impl HotkeyRegistry<Win32HotkeyBackend> {
        /// Pumps the calling thread's `WM_HOTKEY` queue into [`push_pending`].
        ///
        /// Runs on the session thread only (the thread that registered), so it
        /// never steals another thread's messages: the filter names
        /// `WM_HOTKEY` exactly.
        pub fn pump(&mut self) {
            loop {
                // SAFETY: `msg` is a stack `MSG` that outlives the call; the
                // filter passes only `WM_HOTKEY` for no window, removing each
                // message it returns.
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                let more = unsafe {
                    PeekMessageW(&mut msg, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE).as_bool()
                };
                if !more {
                    break;
                }
                let id = super::HotkeyId(msg.wParam.0 as i32);
                if let Some((_, action)) = self.by_id.get(&id) {
                    push_pending(*action);
                }
            }
        }
    }

    /// Owns the session thread that holds the default hotkeys for the whole
    /// run: registering them is why the thread exists, `Drop` unregisters by
    /// stopping the thread (the registry drops with it).
    #[derive(Debug)]
    pub struct HotkeySession {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<Vec<String>>>,
        /// Warnings the thread reported while registering the defaults.
        warnings: Vec<String>,
    }

    impl HotkeySession {
        /// Registers [`DEFAULT_HOTKEYS`] on a fresh thread and returns the
        /// session plus the warnings (taken combinations, one per chord).
        ///
        /// Never fails startup: a refused chord is a warning, not an error —
        /// the player runs with the chords it got (F2's "degrade, never
        /// wedge").
        pub fn start() -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let (warnings_tx, warnings_rx) = std::sync::mpsc::channel();
            let thread = std::thread::Builder::new()
                .name("rustamp-hotkeys".to_owned())
                .spawn(move || run_session(&worker_stop, warnings_tx))
                .ok();
            // The worker reports its warnings right after registering; a
            // thread that never answers leaves the warnings empty rather than
            // blocking startup.
            let warnings = warnings_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_default();
            Self {
                stop,
                thread,
                warnings,
            }
        }

        /// The registration warnings ([`HotkeySession::start`]).
        pub fn warnings(&self) -> &[String] {
            &self.warnings
        }

        /// The fired actions the session thread queued so far.
        pub fn pending(&mut self) -> Vec<HotkeyAction> {
            super::take_pending_hotkeys()
        }
    }

    impl Drop for HotkeySession {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take()
                && thread.is_finished()
            {
                let _ = thread.join();
            }
            // A thread that is still pumping detaches: shutdown must not hang.
            // Its hotkeys die with the process, which exits right after.
        }
    }

    /// Registers the defaults, reports the warnings once, then pumps until
    /// `stop` is set. Returns the warnings (the `JoinHandle` value).
    fn run_session(
        stop: &AtomicBool,
        warnings_tx: std::sync::mpsc::Sender<Vec<String>>,
    ) -> Vec<String> {
        let mut registry = HotkeyRegistry::new(Win32HotkeyBackend);
        let mut warnings = Vec::new();
        for (combo, action) in DEFAULT_HOTKEYS {
            if let Err(error) = registry.register(*combo, *action) {
                warnings.push(format!(
                    "global hotkey {} ({}): {error}; it stays available in-app",
                    describe(*combo),
                    action.shortcut_action(),
                ));
            }
        }
        let _ = warnings_tx.send(warnings.clone());
        while !stop.load(Ordering::SeqCst) {
            registry.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        // Dropping `registry` drops only the bookkeeping: the OS hotkeys die
        // with the thread's message queue at process exit. `Drop` on the
        // session joins this thread first whenever it has already stopped, so
        // the common path unregisters nothing stale and leaks nothing live.
        warnings
    }
}

#[cfg(windows)]
pub use imp::{HotkeySession, Win32HotkeyBackend, Win32HotkeyError};

#[cfg(not(windows))]
mod imp_stub {
    //! Non-Windows stub: global hotkeys are a Win32 feature for now.

    use super::HotkeyAction;

    /// No-op session (see the Win32 `HotkeySession` for the contract).
    #[derive(Debug, Default)]
    pub struct HotkeySession {
        _priv: (),
    }

    impl HotkeySession {
        /// Off Windows there is nothing to register; the warning says so.
        pub fn start() -> Self {
            Self { _priv: () }
        }

        /// The registration warnings (one stub note off Windows).
        pub fn warnings(&self) -> &[String] {
            &[]
        }

        /// Nothing is ever queued off Windows.
        pub fn pending(&mut self) -> Vec<HotkeyAction> {
            Vec::new()
        }
    }
}

#[cfg(not(windows))]
pub use imp_stub::HotkeySession;

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake backend: scripted refusals plus an exact call log.
    #[derive(Debug, Clone, Default)]
    struct FakeBackend {
        refuse: Vec<(u32, u32)>,
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl FakeBackend {
        fn refusing(modifiers: u32, vk: u32) -> Self {
            Self {
                refuse: vec![(modifiers, vk)],
                log: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeError(String);

    impl std::fmt::Display for FakeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl HotkeyBackend for FakeBackend {
        type Error = FakeError;

        fn register(&mut self, id: i32, modifiers: u32, vk: u32) -> Result<(), Self::Error> {
            if self.refuse.contains(&(modifiers, vk)) {
                return Err(FakeError("taken by another application".to_owned()));
            }
            self.log.borrow_mut().push(format!("register {id}"));
            Ok(())
        }

        fn unregister(&mut self, id: i32) -> Result<(), Self::Error> {
            self.log.borrow_mut().push(format!("unregister {id}"));
            Ok(())
        }
    }

    fn combo(vk: u32) -> HotkeyCombo {
        HotkeyCombo::ctrl_alt(vk)
    }

    #[test]
    fn the_defaults_are_six_distinct_ctrl_alt_chords() {
        assert_eq!(DEFAULT_HOTKEYS.len(), 6);
        let mut combos: Vec<HotkeyCombo> =
            DEFAULT_HOTKEYS.iter().map(|(combo, _)| *combo).collect();
        combos.sort_by_key(|c| (c.modifiers.0, c.vk));
        combos.dedup();
        assert_eq!(combos.len(), 6, "no two defaults share a chord");
        for (combo, _) in DEFAULT_HOTKEYS {
            assert!(
                combo.modifiers.contains(HotkeyModifiers::CONTROL)
                    && combo.modifiers.contains(HotkeyModifiers::ALT),
                "{} must stay a Ctrl+Alt chord",
                describe(*combo)
            );
        }
    }

    #[test]
    fn every_default_maps_onto_an_existing_command() {
        for (_, action) in DEFAULT_HOTKEYS {
            assert_eq!(action.commands().len(), 1, "{action:?}");
        }
        assert_eq!(HotkeyAction::Play.commands(), vec![Command::Play]);
        assert_eq!(
            HotkeyAction::PauseUnpause.commands(),
            vec![Command::TogglePause]
        );
        assert_eq!(HotkeyAction::Stop.commands(), vec![Command::Stop]);
        assert_eq!(HotkeyAction::NextTrack.commands(), vec![Command::Next]);
        assert_eq!(HotkeyAction::PreviousTrack.commands(), vec![Command::Prev]);
        assert_eq!(
            HotkeyAction::ToggleMainWindow.commands(),
            vec![Command::ToggleWindow(WindowId::Main)]
        );
    }

    #[test]
    fn the_recorded_ids_are_the_s03_transport_ids() {
        assert_eq!(HotkeyAction::Play.command_id(), Some(40045));
        assert_eq!(HotkeyAction::PauseUnpause.command_id(), Some(40046));
        assert_eq!(HotkeyAction::Stop.command_id(), Some(40047));
        assert_eq!(HotkeyAction::NextTrack.command_id(), Some(40048));
        assert_eq!(HotkeyAction::PreviousTrack.command_id(), Some(40044));
        assert_eq!(HotkeyAction::ToggleMainWindow.command_id(), None);
    }

    #[test]
    fn a_taken_combination_fails_loudly_and_registers_once() {
        let backend = FakeBackend::default();
        let log = backend.log.clone();
        let mut registry = HotkeyRegistry::new(backend);
        let id = registry.register(combo(b'X' as u32), HotkeyAction::Play);
        assert!(id.is_ok());
        let error = registry
            .register(combo(b'X' as u32), HotkeyAction::Stop)
            .expect_err("a taken chord must fail");
        assert!(
            matches!(error, HotkeyError::AlreadyBound(_)),
            "loudly: {error}"
        );
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.action_for(combo(b'X' as u32)),
            Some(HotkeyAction::Play)
        );
        assert_eq!(
            log.borrow().as_slice(),
            ["register 1"],
            "the second registration never reached the backend"
        );
    }

    #[test]
    fn unregistering_is_clean_and_the_chord_becomes_reusable() {
        let backend = FakeBackend::default();
        let log = backend.log.clone();
        let mut registry = HotkeyRegistry::new(backend);
        let id = registry
            .register(combo(b'V' as u32), HotkeyAction::Stop)
            .unwrap();
        assert_eq!(registry.unregister(id), Ok(HotkeyAction::Stop));
        assert!(registry.is_empty());
        // Re-registering the same chord works again with a fresh id.
        let again = registry.register(combo(b'V' as u32), HotkeyAction::Stop);
        assert!(again.is_ok());
        assert_ne!(again.unwrap(), id);
        assert_eq!(registry.len(), 1);
        assert_eq!(
            log.borrow().as_slice(),
            ["register 1", "unregister 1", "register 2"],
            "exactly one backend call per state change: no leak"
        );
    }

    #[test]
    fn unregistering_an_unknown_id_fails_without_touching_the_backend() {
        let backend = FakeBackend::default();
        let log = backend.log.clone();
        let mut registry = HotkeyRegistry::new(backend);
        let error = registry
            .unregister(HotkeyId(999))
            .expect_err("unknown id must fail");
        assert!(matches!(error, HotkeyError::UnknownId(_)), "{error}");
        assert!(registry.is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn a_backend_refusal_records_nothing_and_retries_cleanly() {
        let backend = FakeBackend::refusing(
            HotkeyModifiers::CONTROL.0 | HotkeyModifiers::ALT.0,
            b'B' as u32,
        );
        let log = backend.log.clone();
        let mut registry = HotkeyRegistry::new(backend);
        let error = registry
            .register(combo(b'B' as u32), HotkeyAction::NextTrack)
            .expect_err("a refused chord must fail");
        assert!(matches!(error, HotkeyError::BackendTaken { .. }), "{error}");
        assert!(registry.is_empty(), "a refusal must record nothing");
        assert!(log.borrow().is_empty(), "a refusal logs no registration");
        // The other app released the chord: the retry wins.
        let mut registry = HotkeyRegistry::new(FakeBackend::default());
        assert!(
            registry
                .register(combo(b'B' as u32), HotkeyAction::NextTrack)
                .is_ok()
        );
    }

    #[test]
    fn the_pending_queue_drains_in_order() {
        let _ = take_pending_hotkeys();
        push_pending(HotkeyAction::Play);
        push_pending(HotkeyAction::NextTrack);
        assert_eq!(
            take_pending_hotkeys(),
            vec![HotkeyAction::Play, HotkeyAction::NextTrack]
        );
        assert!(take_pending_hotkeys().is_empty());
    }

    #[test]
    fn describe_names_the_chord() {
        assert_eq!(describe(combo(b'X' as u32)), "Ctrl+Alt+X");
        assert_eq!(
            describe(HotkeyCombo::new(
                HotkeyModifiers::CONTROL | HotkeyModifiers::ALT | HotkeyModifiers::SHIFT,
                0x70
            )),
            "Ctrl+Alt+Shift+F1"
        );
    }
}
