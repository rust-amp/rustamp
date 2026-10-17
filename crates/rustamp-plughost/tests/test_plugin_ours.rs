// This test plugin is OUR OWN code, written for Rustamp.  It is not derived
// from, copied from or based on any Winamp SDK sample, third-party plugin or
// disassembly.  It exercises the host's answering surface (M3a) the way a
// classic plug-in would, but entirely in-process, and asserts the replies a
// plug-in can rely on (S09 / S10).

use rustamp_plughost::{HOST_WINDOW_CLASS, IpcSurface, common, dispatch_message};
use rustamp_plugin_abi::common::WINAMP_VERSION_295;

const IPC_GETVERSION: u32 = 0;
const IPC_ISPLAYING: u32 = 104;
const IPC_SETVOLUME: u32 = 122;
const IPC_GETWND: u32 = 260;

#[test]
fn our_test_plugin_exercises_the_surface_end_to_end() {
    // The host window class is the one S09 fixes; we never invent one.
    assert_eq!(HOST_WINDOW_CLASS, "Winamp v1.x");
    let class_as_str =
        core::str::from_utf8(common::CLASS_MAIN.strip_suffix(b"\0").unwrap()).unwrap();
    assert_eq!(class_as_str, HOST_WINDOW_CLASS);

    let mut surface = IpcSurface::new();

    // A host that has just created its main window publishes the handle the
    // surface reports.  A nonzero placeholder stands in for a real HWND here;
    // the dispatch path itself never opens a window.
    let host_hwnd: u32 = 0x0004_F00D;
    surface.main_hwnd = host_hwnd;

    // 1. Version query: must be the measured 0x2905 (RFC 0009), not 0x2095.
    assert_eq!(
        rustamp_plughost::HostWindow::dispatch_ipc(&mut surface, 0, IPC_GETVERSION),
        WINAMP_VERSION_295
    );

    // 2. Playing state: fresh host reports stopped (0).
    assert_eq!(
        rustamp_plughost::HostWindow::dispatch_ipc(&mut surface, 0, IPC_ISPLAYING),
        0
    );

    // 3. Volume read / write pair (unsigned 0..=255 convention, S10 3.9f).
    // Note: Sent setter return value is an unmeasured fallback convention (S10 §0.2 E4, §13.11).
    assert_eq!(
        rustamp_plughost::HostWindow::dispatch_ipc(&mut surface, 64, IPC_SETVOLUME),
        64
    );
    // Query (-666) reads back the last set value.
    assert_eq!(
        rustamp_plughost::HostWindow::dispatch_ipc(&mut surface, -666, IPC_SETVOLUME),
        64
    );

    // 4. An id the table does not carry falls through to the measured 1.
    assert_eq!(
        rustamp_plughost::HostWindow::dispatch_ipc(&mut surface, 0, 0xDEAD_BEEFu32),
        1
    );

    // 5. Handle selector: IPC_GETWND's selector 0 is the **equalizer** window,
    //    not the main one — that is the mapping the S10 steps measured
    //    (`s10_260_getwnd_eq/pe/mb/video`), and the surface follows it.
    //    (Handles default to 0 on a fresh surface pending M3 architecture decision, S10 §13.8).
    surface.eq_hwnd = host_hwnd;
    assert_eq!(
        rustamp_plughost::HostWindow::dispatch_ipc(&mut surface, 0, IPC_GETWND),
        host_hwnd
    );
}

#[test]
fn dispatch_message_routes_ipc() {
    let mut surface = IpcSurface::new();
    surface.main_hwnd = 7;
    let r = dispatch_message(
        &mut surface,
        0usize,
        common::WM_USER,
        0,
        common::IPC_GETVERSION as i64,
    );
    assert_eq!(r, WINAMP_VERSION_295 as i64);
}
