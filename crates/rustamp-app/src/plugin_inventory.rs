// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The app's record of the plug-ins the plugin host reported.
//!
//! The plugin host answers a load request with
//! [`Message::PluginLoaded`](rustamp_ipc::Message::PluginLoaded), which carries
//! the **host-assigned** handle, the plug-in's display name and its kind, or
//! with [`Message::PluginError`](rustamp_ipc::Message::PluginError), which
//! carries the path that was asked for and the reason. Before this module
//! existed the app read neither: the link's drain ignored every variant but
//! the handshake, so the player never learned a plug-in id and could not name
//! a plug-in in any request (RFC 0020 §3.5).
//!
//! # The ids are the host's, and they die with it
//!
//! The host hands the ids out in load order, starting at one
//! (`rustamp-plughost/src/registry.rs`), and it never repeats one within its
//! own lifetime. A **restarted host re-uses the numbers for different
//! plug-ins**, so an id is valid only for the lifetime of the process that
//! issued it (RFC 0020 Q6). The inventory therefore belongs to one host
//! process: it is cleared whenever the link attaches a new peer or detaches
//! from the old one (see [`crate::host::HostStateLink`]). Nothing here
//! persists an id across a restart, and nothing here invents one.
//!
//! # What the inventory does not know
//!
//! * **Which plug-in is running.** The protocol carries no "a visualization
//!   started" message, so the host's own running-visualization slot
//!   (`PluginRegistry::running_vis`) has no counterpart here. The
//!   visualization plug-ins in the inventory are the ones the host *reported
//!   as loaded*, which is not the same thing as the one that is running.
//! * **Which module of a plug-in.** A visualization plug-in exports several
//!   modules and the wire carries no module list, so a request that names a
//!   module has nothing to name it by yet.
//! * **The path of a loaded plug-in.**
//!   [`Message::PluginLoaded`](rustamp_ipc::Message::PluginLoaded) carries the
//!   name the plug-in reported, not the file it was loaded from; the path is
//!   only in the failure answer, for a plug-in that never loaded.
//!
//! Nothing here guesses: a fact the protocol does not carry stays absent.

use rustamp_plugin_abi::PluginKind;

/// One plug-in the host reported as loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedPlugin {
    /// The handle the host assigned; valid only for that host process's
    /// lifetime (RFC 0020 Q6).
    pub plugin_id: u32,
    /// The display name the plug-in reported.
    pub name: String,
    /// The kind the host recorded for it.
    pub kind: PluginKind,
}

/// One load the host refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLoadFailure {
    /// The path the host was asked to load, exactly as it was sent.
    pub path: String,
    /// The host's description of the failure.
    pub message: String,
}

/// What the host told the app about its plug-ins.
///
/// Filled from the host's answers as they arrive (see
/// [`crate::host::HostStateLink::poll`]) and emptied when the host is
/// replaced, because the ids in it belong to one host process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginInventory {
    /// The loaded plug-ins, in the order the host reported them.
    plugins: Vec<LoadedPlugin>,
    /// The refused loads, in the order the host reported them.
    failures: Vec<PluginLoadFailure>,
}

impl PluginInventory {
    /// Records one plug-in the host reported as loaded.
    ///
    /// The host never repeats an id within its own lifetime, so a repeated id
    /// replaces the earlier entry instead of adding a second one with the
    /// same handle.
    pub fn record_loaded(&mut self, plugin_id: u32, name: String, kind: PluginKind) {
        let entry = LoadedPlugin {
            plugin_id,
            name,
            kind,
        };
        match self.plugins.iter_mut().find(|p| p.plugin_id == plugin_id) {
            Some(slot) => *slot = entry,
            None => self.plugins.push(entry),
        }
    }

    /// Records one load the host refused.
    pub fn record_failure(&mut self, path: String, message: String) {
        self.failures.push(PluginLoadFailure { path, message });
    }

    /// The plug-ins the host reported, in load order.
    pub fn plugins(&self) -> &[LoadedPlugin] {
        &self.plugins
    }

    /// The plug-in with this host-assigned id, if the host reported one.
    pub fn by_id(&self, plugin_id: u32) -> Option<&LoadedPlugin> {
        self.plugins.iter().find(|p| p.plugin_id == plugin_id)
    }

    /// The plug-ins of one kind, in load order.
    ///
    /// "Of this kind" is not "running": the protocol does not say which
    /// plug-in is running (see the module docs).
    pub fn of_kind(&self, kind: PluginKind) -> impl Iterator<Item = &LoadedPlugin> {
        self.plugins.iter().filter(move |p| p.kind == kind)
    }

    /// Takes the refused loads recorded since the last call.
    ///
    /// The app turns them into notes for the user, so each failure is
    /// reported once and then forgotten.
    pub fn take_failures(&mut self) -> Vec<PluginLoadFailure> {
        std::mem::take(&mut self.failures)
    }

    /// Forgets everything: the ids belonged to one host process.
    pub fn clear(&mut self) {
        self.plugins.clear();
        self.failures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_plugin_is_found_by_its_id_and_its_kind() {
        let mut inventory = PluginInventory::default();
        inventory.record_loaded(1, "MilkDrop 2".to_string(), PluginKind::Visualization);
        inventory.record_loaded(2, "Sleeper".to_string(), PluginKind::Dsp);

        assert_eq!(inventory.plugins().len(), 2);
        assert!(!inventory.plugins().is_empty());
        let vis = inventory.by_id(1).expect("the vis plug-in is recorded");
        assert_eq!(vis.name, "MilkDrop 2");
        assert_eq!(vis.kind, PluginKind::Visualization);
        assert_eq!(inventory.by_id(99), None);
        let of_kind: Vec<&LoadedPlugin> = inventory.of_kind(PluginKind::Visualization).collect();
        assert_eq!(of_kind.len(), 1);
        assert_eq!(of_kind[0].plugin_id, 1);
    }

    #[test]
    fn a_repeated_id_replaces_the_entry_instead_of_duplicating_it() {
        let mut inventory = PluginInventory::default();
        inventory.record_loaded(7, "first".to_string(), PluginKind::General);
        inventory.record_loaded(7, "second".to_string(), PluginKind::Input);

        assert_eq!(inventory.plugins().len(), 1);
        let entry = inventory.by_id(7).expect("the plug-in is recorded");
        assert_eq!(entry.name, "second");
        assert_eq!(entry.kind, PluginKind::Input);
    }

    #[test]
    fn failures_are_taken_once_and_clearing_forgets_everything() {
        let mut inventory = PluginInventory::default();
        inventory.record_loaded(1, "ok".to_string(), PluginKind::General);
        inventory.record_failure("C:\\plugins\\bad.dll".to_string(), "not a PE".to_string());

        let failures = inventory.take_failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, "C:\\plugins\\bad.dll");
        assert_eq!(failures[0].message, "not a PE");
        // The failure is reported once, not on every poll.
        assert!(inventory.take_failures().is_empty());

        inventory.clear();
        assert!(inventory.plugins().is_empty());
        assert_eq!(inventory.plugins().len(), 0);
        assert_eq!(inventory.by_id(1), None);
    }
}
