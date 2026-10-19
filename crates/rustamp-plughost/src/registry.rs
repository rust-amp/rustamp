//! The host's loaded-plugin registry and its control-channel routing
//! (`docs/tasks/P3-dll-acceptance.md` §5.3, "Control routing").
//!
//! The app asks for a load with [`Message::LoadPlugin`] and the host answers
//! with [`Message::PluginLoaded`] or [`Message::PluginError`]. All three
//! variants already exist in `rustamp-ipc` with their directions documented,
//! so **this module changes no wire format and needs no RFC**. What it adds is
//! the host side that P3 §5.3 asked for: dispatch the request on the UI
//! thread, report the *actual* loader error rather than a generic failure, and
//! answer through the framed channel rather than printing (a stray line on
//! stdout would corrupt the frame stream — see `src/main.rs`).
//!
//! # What this module does not decide
//!
//! * **Approval.** The registry never approves anything. It holds the
//!   [`ApprovalStore`] the operator supplied and passes it to the loader,
//!   which requires full-byte equality against it. With no store the store is
//!   empty, which approves nothing; with a malformed store the registry is
//!   [`Approvals::Unavailable`] and refuses every request with the reason.
//!   **A missing or broken store is never an empty allowlist by accident.**
//! * **`PluginLoaded` is not acceptance.** P3 §7 lists it among the false
//!   passes. It records that the real loader mapped approved bytes, resolved
//!   the kind's export and ran the getter; it proves nothing about windows,
//!   dialogs, audio or any other plugin kind.
//!
//! # Threading and lifetime
//!
//! Every method runs on the UI thread that owns the [`HostWindow`]: a plugin's
//! `init`/`config`/`quit` run there (S09 §3.5, §7.2), and `HostWindow` is
//! neither `Send` nor `Sync`, so holding one is the proof. The registry quits
//! its plugins in **reverse load order** (S09 §3.5 line 166, §7.2 line 380) and
//! never frees a DLL (S09 §1 line 46): a plugin may have subclassed the host
//! window or spawned threads. The host window must therefore outlive the
//! registry.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::path::Path;

use rustamp_ipc::Message;
use rustamp_plugin_abi::PluginKind;

use crate::approval::ApprovalStore;
use crate::host_window::HostWindow;
use crate::loader::{LoadedPlugin, PluginConfigError, load_plugin_with_store};

/// Where the registry's approval decisions come from.
enum Approvals {
    /// The operator's allowlist. An empty one approves nothing.
    Store(ApprovalStore),
    /// The allowlist could not be read. Every load is refused with this reason;
    /// it is deliberately **not** treated as "nothing approved yet", so a
    /// broken store can never look like a working one.
    Unavailable(String),
}

/// One plugin the host holds, with the id the protocol hands back to the app.
struct LoadedEntry {
    id: u32,
    plugin: LoadedPlugin,
}

/// Plugins this host has loaded, in load order.
pub struct PluginRegistry {
    approvals: Approvals,
    loaded: Vec<LoadedEntry>,
    next_id: u32,
}

/// Why a configuration request did not reach a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryConfigError {
    /// No plugin carries this id.
    UnknownPlugin(u32),
    /// The plugin refused the call; nothing was invoked.
    Plugin(PluginConfigError),
}

impl std::fmt::Display for RegistryConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPlugin(id) => write!(f, "no plugin with id {id} is loaded"),
            Self::Plugin(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RegistryConfigError {}

/// Is a plugin of kind `actual` what the request asked for?
///
/// Pure and deliberately separate from the load: it is applied twice — once on
/// the kind passive preflight selects, before anything is mapped, and once on
/// the kind the loader actually produced, which is the authoritative answer
/// (the pre-check reads its own copy of the file and cannot bind the bytes the
/// loader later seals). Returns the message the `PluginError` carries.
pub fn accept_loaded_kind(expected: PluginKind, actual: PluginKind) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "plugin is a {actual:?} plugin (export {}), but the request asked for {expected:?}",
            String::from_utf8_lossy(
                actual
                    .export_name()
                    .strip_suffix(&[0])
                    .unwrap_or(actual.export_name())
            )
        ))
    }
}

impl PluginRegistry {
    /// A registry over the operator's allowlist.
    pub fn new(store: ApprovalStore) -> Self {
        Self {
            approvals: Approvals::Store(store),
            loaded: Vec::new(),
            next_id: 1,
        }
    }

    /// A registry that refuses every load, naming why the allowlist is missing.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            approvals: Approvals::Unavailable(reason.into()),
            loaded: Vec::new(),
            next_id: 1,
        }
    }

    /// Read the allowlist from `path`, or an empty one when no path was given.
    ///
    /// `None` means the operator named no store: the result approves nothing,
    /// which is the safe default, not an error. A path that cannot be read or
    /// parsed yields [`Self::unavailable`] — never a silent empty allowlist.
    pub fn from_store_path(path: Option<&Path>) -> Self {
        match path {
            None => Self::new(ApprovalStore::new()),
            Some(path) => match ApprovalStore::load_from_file(path) {
                Ok(store) => Self::new(store),
                Err(error) => {
                    Self::unavailable(format!("{} could not be read: {error}", path.display()))
                }
            },
        }
    }

    /// How many entries the allowlist holds, or `None` when it is unavailable.
    pub fn approved_count(&self) -> Option<usize> {
        match &self.approvals {
            Approvals::Store(store) => Some(store.len()),
            Approvals::Unavailable(_) => None,
        }
    }

    /// The ids of the loaded plugins, in load order.
    pub fn loaded_ids(&self) -> Vec<u32> {
        self.loaded.iter().map(|entry| entry.id).collect()
    }

    /// Handle one [`Message::LoadPlugin`], returning the reply to send back.
    ///
    /// The order is the point: the allowlist must exist, the kind the request
    /// asked for must match the kind the passive preflight selects (so a
    /// wrong-kind request never reaches `LoadLibraryExW` and never runs a
    /// `DllMain`), only then does the loader seal and map the approved bytes,
    /// and finally the kind the loader actually produced is checked again.
    ///
    /// # Safety
    ///
    /// Must be called on the UI thread owning `window`. The plugin bytes the
    /// allowlist approves, and their transitive dependencies, must be code the
    /// operator explicitly trusts to execute in this process: approval decides
    /// that, and neither the preflight nor this routing can establish it.
    pub unsafe fn handle_load_request(
        &mut self,
        path: &str,
        expected: PluginKind,
        window: &HostWindow,
    ) -> Message {
        let error = |message: String| Message::PluginError {
            path: path.to_string(),
            message,
        };

        let store = match &self.approvals {
            Approvals::Unavailable(reason) => {
                return error(format!(
                    "no plugin is approved because the approval store is unavailable: {reason}"
                ));
            }
            Approvals::Store(store) => store,
        };

        // Pre-mapping kind check on our own read of the candidate. This read is
        // not the approval basis and not the bytes that get loaded; the loader
        // re-reads and seals its own copy. Its only job is to refuse an
        // obviously wrong-kind request before any plugin code can run.
        let candidate = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => return error(format!("plugin image could not be read: {e}")),
        };
        let selected = match crate::pe_preflight::check_pe_i386_dll(&candidate) {
            Ok(kind) => kind,
            Err(e) => return error(format!("PE DLL preflight refused the plugin: {e}")),
        };
        if let Err(message) = accept_loaded_kind(expected, selected) {
            return error(format!("{message}; refused before any mapping"));
        }
        drop(candidate);

        // SAFETY: the caller's UI-thread and approved-code invariants are
        // forwarded unchanged; the store is the operator's approval decision.
        let loaded = match unsafe { load_plugin_with_store(Path::new(path), store, window) } {
            Ok(loaded) => loaded,
            Err(e) => return error(e.to_string()),
        };

        // Authoritative check: this is the kind of the image that was actually
        // sealed, mapped and called.
        if let Err(message) = accept_loaded_kind(expected, loaded.kind()) {
            let mut loaded = loaded;
            loaded.quit();
            return error(format!("{message}; the loaded plugin was quit again"));
        }

        let id = self.next_id;
        self.next_id += 1;
        let reply = Message::PluginLoaded {
            plugin_id: id,
            name: loaded.description().to_string(),
            kind: loaded.kind(),
        };
        self.loaded.push(LoadedEntry { id, plugin: loaded });
        reply
    }

    /// Open the configuration dialog of the plugin with this id, on this thread
    /// (P3 §4 step 9). Synchronous: a modal dialog returns only when the plugin
    /// closes it.
    pub fn config(&self, plugin_id: u32) -> Result<(), RegistryConfigError> {
        let entry = self
            .loaded
            .iter()
            .find(|entry| entry.id == plugin_id)
            .ok_or(RegistryConfigError::UnknownPlugin(plugin_id))?;
        entry.plugin.config().map_err(RegistryConfigError::Plugin)
    }

    /// What the plugin with this id reports about itself, if it is a general
    /// plugin ([`LoadedPlugin::general_info`]).
    pub fn general_info(&self, plugin_id: u32) -> Option<crate::loader::GeneralModuleInfo> {
        self.loaded
            .iter()
            .find(|entry| entry.id == plugin_id)?
            .plugin
            .general_info()
    }

    /// Quit every loaded plugin in **reverse** load order (S09 §3.5 line 166,
    /// §7.2 line 380) and return the ids in the order they were quit.
    ///
    /// The DLLs stay mapped (S09 §1 line 46). Idempotent: a second call quits
    /// nothing again, because [`LoadedPlugin::quit`] is.
    pub fn quit_all(&mut self) -> Vec<u32> {
        let mut order = Vec::with_capacity(self.loaded.len());
        for entry in self.loaded.iter_mut().rev() {
            entry.plugin.quit();
            order.push(entry.id);
        }
        order
    }

    /// Take ownership of a plugin that was loaded elsewhere, for tests of the
    /// registry's ordering and dispatch that must not map a DLL.
    #[cfg(test)]
    fn insert_for_test(&mut self, plugin: LoadedPlugin) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.loaded.push(LoadedEntry { id, plugin });
        id
    }
}

impl Drop for PluginRegistry {
    fn drop(&mut self) {
        // Reverse order here too: dropping the Vec would quit in load order.
        self.quit_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pe_preflight::test_dll;
    use rustamp_plugin_abi::general::GeneralPlugin;

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            use std::io::Write;
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "rustamp-registry-test-{}-{}.dll",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            Self(path)
        }

        fn as_str(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn error_message(reply: &Message) -> &str {
        match reply {
            Message::PluginError { message, .. } => message,
            other => panic!("expected a PluginError, got {other:?}"),
        }
    }

    // ---- synthetic general plugins, for ordering and dispatch -------------

    thread_local! {
        static QUIT_LOG: std::cell::RefCell<Vec<&'static str>> = const {
            std::cell::RefCell::new(Vec::new())
        };
        static CONFIG_LOG: std::cell::RefCell<Vec<&'static str>> = const {
            std::cell::RefCell::new(Vec::new())
        };
    }

    unsafe extern "C" fn quit_a() {
        QUIT_LOG.with(|log| log.borrow_mut().push("a"));
    }
    unsafe extern "C" fn quit_b() {
        QUIT_LOG.with(|log| log.borrow_mut().push("b"));
    }
    unsafe extern "C" fn quit_c() {
        QUIT_LOG.with(|log| log.borrow_mut().push("c"));
    }
    unsafe extern "C" fn config_a() {
        CONFIG_LOG.with(|log| log.borrow_mut().push("a"));
    }

    fn general(
        config: Option<unsafe extern "C" fn()>,
        quit: Option<unsafe extern "C" fn()>,
    ) -> &'static mut GeneralPlugin {
        // Leaked on purpose: the module must outlive the registry that holds a
        // raw pointer to it, exactly as a mapped DLL's static data does.
        Box::leak(Box::new(GeneralPlugin {
            version: rustamp_plugin_abi::common::GEN_VERSION,
            description: c"synthetic general".as_ptr(),
            config,
            quit,
            ..GeneralPlugin::default()
        }))
    }

    #[test]
    fn an_unavailable_approval_store_refuses_every_load_with_its_reason() {
        let fixture = Fixture::new(&test_dll(0x014c));
        let mut registry =
            PluginRegistry::unavailable("plugins-approved.toml: line 3 is malformed");
        assert_eq!(registry.approved_count(), None);
        // SAFETY: the refusal happens before any approval or mapping.
        let reply = unsafe {
            registry.handle_load_request(fixture.as_str(), PluginKind::Input, &HostWindow::dummy())
        };
        let message = error_message(&reply);
        assert!(
            message.contains("approval store is unavailable")
                && message.contains("line 3 is malformed"),
            "the reason must survive into the reply: {message}"
        );
        assert!(registry.loaded_ids().is_empty());
    }

    #[test]
    fn an_empty_store_approves_nothing_even_for_a_structurally_valid_dll() {
        let fixture = Fixture::new(&test_dll(0x014c));
        // The candidate passes passive preflight as an input plugin...
        assert_eq!(
            crate::pe_preflight::check_pe_i386_dll(&test_dll(0x014c)),
            Ok(PluginKind::Input)
        );
        let mut registry = PluginRegistry::new(ApprovalStore::new());
        assert_eq!(registry.approved_count(), Some(0));
        // ... and is still refused, because preflight is not approval.
        // SAFETY: an empty allowlist refuses before any mapping.
        let reply = unsafe {
            registry.handle_load_request(fixture.as_str(), PluginKind::Input, &HostWindow::dummy())
        };
        let message = error_message(&reply);
        assert!(
            message.contains("do not match the approved image"),
            "expected the loader's own refusal: {message}"
        );
        assert!(registry.loaded_ids().is_empty());
    }

    #[test]
    fn a_wrong_expected_kind_is_refused_before_any_mapping() {
        // The fixture exports winampGetInModule2; the request asks for a
        // general plugin. Approve the bytes, so that nothing but the kind can
        // be what refuses this.
        let bytes = test_dll(0x014c);
        let fixture = Fixture::new(&bytes);
        let mut store = ApprovalStore::new();
        store.approve_bytes("test.dll", &bytes).unwrap();
        let mut registry = PluginRegistry::new(store);
        // SAFETY: the kind mismatch refuses before any mapping.
        let reply = unsafe {
            registry.handle_load_request(
                fixture.as_str(),
                PluginKind::General,
                &HostWindow::dummy(),
            )
        };
        let message = error_message(&reply);
        assert!(
            message.contains("refused before any mapping")
                && message.contains("asked for General")
                && message.contains("winampGetInModule2"),
            "the reply must name both kinds and say nothing ran: {message}"
        );
        assert!(registry.loaded_ids().is_empty());
    }

    #[test]
    fn an_output_candidate_is_refused_when_another_kind_was_asked_for() {
        // The fixture exports winampGetOutModule (S09 §2); the request asks
        // for an input plugin. Approve the bytes, so that nothing but the
        // kind can be what refuses this.
        let mut bytes = test_dll(0x014c);
        let name = b"winampGetOutModule\0";
        bytes[0x260..0x260 + name.len()].copy_from_slice(name);
        assert_eq!(
            crate::pe_preflight::check_pe_i386_dll(&bytes),
            Ok(PluginKind::Output)
        );
        let fixture = Fixture::new(&bytes);
        let mut store = ApprovalStore::new();
        store.approve_bytes("out_test.dll", &bytes).unwrap();
        let mut registry = PluginRegistry::new(store);
        // SAFETY: the kind mismatch refuses before any mapping.
        let reply = unsafe {
            registry.handle_load_request(fixture.as_str(), PluginKind::Input, &HostWindow::dummy())
        };
        let message = error_message(&reply);
        assert!(
            message.contains("refused before any mapping")
                && message.contains("asked for Input")
                && message.contains("winampGetOutModule"),
            "the reply must name both kinds and say nothing ran: {message}"
        );
        assert!(registry.loaded_ids().is_empty());
    }

    #[test]
    fn a_visualization_candidate_is_refused_when_another_kind_was_asked_for() {
        // The fixture exports winampVisGetHeader (S09 §2); the request asks
        // for an input plugin. Approve the bytes, so that nothing but the
        // kind can be what refuses this.
        let mut bytes = test_dll(0x014c);
        let name = b"winampVisGetHeader\0";
        bytes[0x260..0x260 + name.len()].copy_from_slice(name);
        assert_eq!(
            crate::pe_preflight::check_pe_i386_dll(&bytes),
            Ok(PluginKind::Visualization)
        );
        let fixture = Fixture::new(&bytes);
        let mut store = ApprovalStore::new();
        store.approve_bytes("vis_test.dll", &bytes).unwrap();
        let mut registry = PluginRegistry::new(store);
        // SAFETY: the kind mismatch refuses before any mapping.
        let reply = unsafe {
            registry.handle_load_request(fixture.as_str(), PluginKind::Input, &HostWindow::dummy())
        };
        let message = error_message(&reply);
        assert!(
            message.contains("refused before any mapping")
                && message.contains("asked for Input")
                && message.contains("winampVisGetHeader"),
            "the reply must name both kinds and say nothing ran: {message}"
        );
        assert!(registry.loaded_ids().is_empty());
    }

    #[test]
    fn a_missing_file_reports_the_real_io_error() {
        let mut registry = PluginRegistry::new(ApprovalStore::new());
        let missing = std::env::temp_dir().join("rustamp-registry-no-such-plugin.dll");
        // SAFETY: nothing is mapped; the read fails first.
        let reply = unsafe {
            registry.handle_load_request(
                missing.to_str().unwrap(),
                PluginKind::Input,
                &HostWindow::dummy(),
            )
        };
        let message = error_message(&reply);
        assert!(
            message.contains("could not be read"),
            "the I/O error must not be swallowed: {message}"
        );
        match &reply {
            Message::PluginError { path, .. } => {
                assert_eq!(
                    path,
                    missing.to_str().unwrap(),
                    "the reply names the request"
                )
            }
            other => panic!("expected PluginError, got {other:?}"),
        }
    }

    #[test]
    fn accept_loaded_kind_is_authoritative_after_the_load() {
        assert_eq!(
            accept_loaded_kind(PluginKind::General, PluginKind::General),
            Ok(())
        );
        for (expected, actual) in [
            (PluginKind::General, PluginKind::Input),
            (PluginKind::Input, PluginKind::Dsp),
            (PluginKind::Dsp, PluginKind::Visualization),
            (PluginKind::Input, PluginKind::Output),
            (PluginKind::Output, PluginKind::General),
        ] {
            let message = accept_loaded_kind(expected, actual)
                .expect_err("a mismatch must never be accepted");
            assert!(message.contains(&format!("{actual:?}")));
            assert!(message.contains(&format!("{expected:?}")));
        }
    }

    #[test]
    fn plugins_quit_in_reverse_load_order() {
        QUIT_LOG.with(|log| log.borrow_mut().clear());
        let mut registry = PluginRegistry::new(ApprovalStore::new());
        let a = registry.insert_for_test(LoadedPlugin::general_for_test(
            "gen_a",
            general(None, Some(quit_a)),
            true,
            0,
        ));
        let b = registry.insert_for_test(LoadedPlugin::general_for_test(
            "gen_b",
            general(None, Some(quit_b)),
            true,
            0,
        ));
        let c = registry.insert_for_test(LoadedPlugin::general_for_test(
            "gen_c",
            general(None, Some(quit_c)),
            true,
            0,
        ));
        assert_eq!(registry.loaded_ids(), vec![a, b, c]);

        assert_eq!(registry.quit_all(), vec![c, b, a]);
        assert_eq!(
            QUIT_LOG.with(|log| log.borrow().clone()),
            vec!["c", "b", "a"],
            "the plugins themselves must see reverse load order"
        );

        // Idempotent, including through Drop.
        assert_eq!(registry.quit_all(), vec![c, b, a]);
        drop(registry);
        assert_eq!(
            QUIT_LOG.with(|log| log.borrow().clone()),
            vec!["c", "b", "a"],
            "no plugin may be quit twice"
        );
    }

    #[test]
    fn config_reaches_exactly_the_named_plugin_and_an_unknown_id_touches_nothing() {
        CONFIG_LOG.with(|log| log.borrow_mut().clear());
        let mut registry = PluginRegistry::new(ApprovalStore::new());
        let with_config = registry.insert_for_test(LoadedPlugin::general_for_test(
            "gen_with_config",
            general(Some(config_a), None),
            true,
            0,
        ));
        let without = registry.insert_for_test(LoadedPlugin::general_for_test(
            "gen_without_config",
            general(None, None),
            true,
            0,
        ));

        assert_eq!(registry.config(with_config), Ok(()));
        assert_eq!(CONFIG_LOG.with(|log| log.borrow().clone()), vec!["a"]);

        assert_eq!(
            registry.config(without),
            Err(RegistryConfigError::Plugin(
                PluginConfigError::MissingCallback
            ))
        );
        assert_eq!(
            registry.config(9_999),
            Err(RegistryConfigError::UnknownPlugin(9_999))
        );
        assert_eq!(
            CONFIG_LOG.with(|log| log.borrow().clone()),
            vec!["a"],
            "only the plugin that has a config slot may have been called"
        );

        // And the info an acceptance runner checks before calling anything.
        let info = registry
            .general_info(without)
            .expect("a general plugin reports its slots");
        assert!(!info.has_config && !info.has_quit);
        assert!(registry.general_info(9_999).is_none());
    }

    #[test]
    fn a_fresh_registry_holds_nothing_and_hands_out_ids_from_one() {
        let registry = PluginRegistry::new(ApprovalStore::new());
        assert!(registry.loaded_ids().is_empty());
        assert_eq!(registry.next_id, 1, "0 is not handed out as a plugin id");
        assert_eq!(registry.approved_count(), Some(0));
    }
}
