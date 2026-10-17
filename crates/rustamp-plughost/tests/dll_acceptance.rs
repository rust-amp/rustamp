//! Acceptance test loading a real 32-bit Winamp plugin DLL into the Rustamp host.
//!
//! Gated on the Hyper-V test VM ([`rustamp_plughost::require_test_vm`]).
//! Loads `in_openmpt.dll` (a freely licensed libopenmpt 0.8.9 BSD-3-Clause build),
//! validates the module via passive PE preflight, maps the DLL, resolves the entry
//! point `winampGetInModule2`, fills host-provided fields, calls `init()`, and proves
//! that:
//! 1. The entry point ran and returned a valid module with a matching description.
//! 2. The plugin sent messages to the host's Win32 main window (`Winamp v1.x`).
//! 3. The host answered at least one query from the plugin (e.g. `IPC_GETVERSION`).
//!
//! Must refuse loudly (not skip silently) when either the VM or the plugin binary is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::path::PathBuf;

use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::load_plugin;
use rustamp_plughost::message_loop;
use rustamp_plugin_abi::PluginKind;

#[cfg(windows)]
fn mapped_path(hinstance: *mut core::ffi::c_void) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleFileNameW(module: *mut core::ffi::c_void, path: *mut u16, size: u32) -> u32;
    }
    let mut path = vec![0_u16; 32_768];
    // SAFETY: instance belongs to our retained DLL, buffer is writable for size.
    let len = unsafe { GetModuleFileNameW(hinstance, path.as_mut_ptr(), path.len() as u32) };
    assert!(
        len != 0 && (len as usize) < path.len(),
        "mapped DLL path is missing/truncated"
    );
    PathBuf::from(std::ffi::OsString::from_wide(&path[..len as usize]))
}

/// Find `in_openmpt.dll` across environment and staged locations, or refuse loudly.
fn find_in_openmpt_dll() -> PathBuf {
    // 1. Explicit environment variable (e.g. set by test runner)
    if let Ok(path_str) = std::env::var("RUSTAMP_TESTPLUGINS") {
        let p = PathBuf::from(&path_str);
        if p.is_file() {
            return p;
        }
        let cand1 = p.join("in_openmpt.dll");
        if cand1.is_file() {
            return cand1;
        }
        let cand2 = p.join("libopenmpt").join("Winamp").join("in_openmpt.dll");
        if cand2.is_file() {
            return cand2;
        }
    }

    // 2. Standard guest VM staged paths
    let guest1 = PathBuf::from(r"C:\rustamp\testplugins\libopenmpt\Winamp\in_openmpt.dll");
    if guest1.is_file() {
        return guest1;
    }
    let guest2 = PathBuf::from(r"C:\rustamp\testplugins\in_openmpt.dll");
    if guest2.is_file() {
        return guest2;
    }

    // 3. Orchestrator extracted path on reference/test drive
    let ref_path = PathBuf::from(r"D:\code\rustamp-testplugins\libopenmpt\Winamp\in_openmpt.dll");
    if ref_path.is_file() {
        return ref_path;
    }

    panic!(
        "REFUSING LOUDLY: missing plugin binary in_openmpt.dll. \
         Checked RUSTAMP_TESTPLUGINS, {}, {}, and {}. \
         DLL acceptance tests must refuse loudly by design when reference binaries are missing.",
        guest1.display(),
        guest2.display(),
        ref_path.display()
    );
}

#[test]
#[ignore = "VM-only: loads real 32-bit in_openmpt.dll and creates real Win32 host window; run in test VM"]
fn loads_real_32bit_in_openmpt_dll_and_proves_host_interaction() {
    // 1. Gate on the Hyper-V test VM before any window or DLL is loaded.
    // Refuses loudly if this machine is not the test VM.
    require_test_vm();

    // 2. Refuse loudly if running in a 64-bit process: 32-bit DLLs can only map into i686.
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: loading 32-bit plugin DLL requires i686 target (32-bit x86); \
             cannot map 32-bit DLL into a 64-bit process"
        );
    }

    // 3. Locate the approved plugin binary or refuse loudly.
    let plugin_path = find_in_openmpt_dll();
    // This VM-only acceptance runner trusts the owner-approved staged fixture.
    // It is not a production approval policy for an arbitrary supplied pathname.
    let approved_bytes = std::fs::read(&plugin_path).expect("approved VM fixture must be readable");
    assert!(
        plugin_path.is_file(),
        "plugin binary must exist at {}",
        plugin_path.display()
    );

    // 4. Create the real host Win32 window (Winamp v1.x class).
    let surface = IpcSurface::new();
    let window = HostWindow::create(surface).expect("HostWindow creation failed in test VM");
    assert_ne!(
        window.hwnd_raw(),
        0,
        "HostWindow HWND must be non-zero after creation"
    );

    // 5. Load the real 32-bit plugin DLL via the clean-room loader.
    // SAFETY: running inside the gated test VM on i686 with approved in_openmpt.dll.
    let mut loaded = unsafe {
        load_plugin(&plugin_path, &approved_bytes, &window)
            .expect("failed to load real 32-bit in_openmpt.dll into host")
    };

    // P5: prove that the real mapping uses a different, protected snapshot whose
    // complete bytes equal the approved source, not just its architecture header.
    #[cfg(windows)]
    {
        let mapped = std::fs::canonicalize(mapped_path(loaded.hinstance())).unwrap();
        let source = std::fs::canonicalize(&plugin_path).unwrap();
        assert_eq!(loaded.path(), source);
        assert_ne!(mapped, source, "loader must not map the original pathname");
        assert_eq!(std::fs::read(&mapped).unwrap(), approved_bytes);
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&mapped)
                .is_err(),
            "mapped snapshot must remain write-locked"
        );
    }
    #[cfg(not(windows))]
    let _ = approved_bytes;

    // 6. Prove the entry point ran:
    // - Kind must be Input.
    // - Description must be non-empty and identify the module.
    assert_eq!(loaded.kind(), PluginKind::Input);
    assert!(
        !loaded.description().is_empty(),
        "plugin description must not be empty"
    );
    let desc = loaded.description().to_ascii_lowercase();
    assert!(
        desc.contains("openmpt") || desc.contains("module"),
        "unexpected plugin description: '{}'",
        loaded.description()
    );
    assert!(loaded.is_initialized(), "plugin must be marked initialized");

    // 7. Drain the message loop to process any queued messages.
    message_loop::drain(&window);

    // 8. Prove the plugin's own message arrived, and the host answered at least one query.
    let queries = window.recorded_ipc_queries();
    let messages = window.recorded_messages();

    assert!(
        !queries.is_empty() || !messages.is_empty(),
        "plugin must have sent messages to the host window during init or pumping"
    );

    assert!(
        !queries.is_empty(),
        "host must have answered at least one IPC query from in_openmpt.dll (e.g. IPC_GETVERSION)"
    );

    // Verify the query results: version query 0 must return WINAMP_VERSION_295 = 0x2905.
    for (ipc_id, wparam, reply) in &queries {
        println!("Recorded IPC query: id={ipc_id}, wparam={wparam} -> reply={reply:#x}");
        if *ipc_id == rustamp_plugin_abi::common::IPC_GETVERSION {
            assert_eq!(
                *reply,
                rustamp_plugin_abi::common::WINAMP_VERSION_295,
                "IPC_GETVERSION (0) query from in_openmpt.dll must receive 0x2905"
            );
        }
    }

    // 9. Clean shutdown: invoke quit() and destroy the host window.
    loaded.quit();
    window.destroy();
}
