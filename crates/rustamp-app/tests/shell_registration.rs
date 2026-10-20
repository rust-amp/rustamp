// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The shell registration, written for real: **VM-only, `#[ignore]`d**.
//!
//! The registration model (the table of keys and verbs) is unit-tested
//! headlessly in `rustamp-platform`; the Win32 half that writes it needs a
//! registry, so it is exercised here, inside the headless test VM only:
//!
//! ```text
//! cargo test -p rustamp-app --test shell_registration -- --ignored
//! ```
//!
//! The gate is the shared one ([`rustamp_app::window::require_test_vm`]): the
//! test writes to `HKEY_CURRENT_USER\Software\Classes` and must never do that
//! on the host. The test removes the registration again before it ends.

use rustamp_platform::shell_registration;

/// Writes the registration and reads it back.
#[test]
#[ignore = "VM-only: writes the registry; run inside the headless test VM"]
fn the_registration_lands_in_the_registry_and_can_be_removed_again() {
    // The gate, before anything is written.
    rustamp_app::window::require_test_vm();

    let exe = std::env::current_exe().expect("the test binary's own path");
    let exe = exe.to_string_lossy().to_string();

    shell_registration::install(std::path::Path::new(&exe)).expect("write the registration");

    // The `open` verb of the media handler (S13 §3).
    let command = shell_registration::registered_value(r"Rustamp.File\shell\open\command")
        .expect("the open command must be registered");
    assert!(
        command.contains(&exe),
        "the command must name the player: {command}"
    );

    // The bookmark verb carries the form S13 §1 records.
    let bookmark = shell_registration::registered_value(r"Rustamp.File\shell\ListBookmark\command")
        .expect("the ListBookmark command must be registered");
    assert!(
        bookmark.contains("/BOOKMARK"),
        "the bookmark verb must carry the bookmark form: {bookmark}"
    );

    // The `DropTarget` values of `Play` and `Enqueue` (S13 §3).
    for verb in ["Play", "Enqueue"] {
        assert!(
            shell_registration::registered_value(&format!(r"Rustamp.File\shell\{verb}\DropTarget"))
                .is_some(),
            "the {verb} verb must carry a DropTarget value"
        );
    }

    // The skin archive's one verb (S13 §3).
    assert!(
        shell_registration::registered_value(r"Rustamp.SkinZip\shell\install\command").is_some(),
        "the skin handler must carry its install verb"
    );

    // The directory's two verbs (S13 §3).
    for verb in ["Play", "Enqueue"] {
        assert!(
            shell_registration::registered_value(&format!(r"Directory\shell\{verb}\command"))
                .is_some(),
            "the directory handler must carry the {verb} verb"
        );
    }

    // Registering again changes nothing (the idempotence reading of S13 §3).
    shell_registration::install(std::path::Path::new(&exe)).expect("write the registration again");

    shell_registration::uninstall().expect("remove the registration");
    assert!(
        shell_registration::registered_value(r"Rustamp.File\shell\open\command").is_none(),
        "the removal must take the verb keys with it"
    );
}
