//! VM-only acceptance for two paths of the plug-in host that no headless test
//! can reach: a library that refuses a new system until it is told an old one
//! (`vis_nsfs`), and a frame a visualization plug-in embeds its window in, drawn
//! from the sheet the player sends (`vis_avs`).
//!
//! Both load a real, third-party 32-bit DLL into this process, so they run only
//! in the headless test VM and only in an i686 test binary. They are
//! `#[ignore]`d by default and refuse loudly, never skip, when the machine or
//! the staged plug-ins are not there:
//!
//! ```text
//! cargo test -p rustamp-plughost --test helper_acceptance --target i686-pc-windows-msvc -- --ignored
//! ```
//!
//! What they do **not** cover: the audio path (these two plug-ins have none), the
//! frame's own chrome under a real skin (the sheet here is synthetic, so only the
//! frame's kind and its style are asserted, not the pixels a player's skin draws),
//! and every other third-party visualizer.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![cfg(windows)]

use std::path::{Path, PathBuf};

use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::embed_frame;
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::load_plugin_with_store;
use rustamp_plugin_abi::common::CLASS_GEN;

use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, FindWindowW, GWL_STYLE, GetWindowLongW, IsWindow, WS_CAPTION, WS_POPUP,
};
use windows::core::PCWSTR;

/// The staged plug-ins: the VM runner lays them out here, folder by folder.
fn testplugins() -> PathBuf {
    std::env::var_os("RUSTAMP_TESTPLUGINS")
        .map(PathBuf::from)
        .into_iter()
        .chain([
            PathBuf::from(r"C:\rustamp\testplugins"),
            PathBuf::from(r"D:\code\rustamp-testplugins"),
        ])
        .find(|dir| dir.is_dir())
        .expect("REFUSING LOUDLY: the staged plug-ins were not found (set RUSTAMP_TESTPLUGINS)")
}

/// The staged file called `name`, in whichever folder the runner put it.
fn staged(name: &str) -> PathBuf {
    let mut found = Vec::new();
    let mut pending = vec![testplugins()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|file| file.to_string_lossy().eq_ignore_ascii_case(name))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    assert!(
        !found.is_empty(),
        "REFUSING LOUDLY: {name} is not staged under {}",
        testplugins().display()
    );
    found.remove(0)
}

/// The interface: a 32-bit plug-in loads only into an i686 test binary.
fn require_i686() {
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: loading a 32-bit plug-in DLL requires an i686 test binary, \
             but this one is {}",
            std::env::consts::ARCH
        );
    }
}

/// A store that approves exactly this image.
fn approved(dll: &Path) -> ApprovalStore {
    let bytes = std::fs::read(dll).expect("reading the plug-in");
    let name = dll.file_name().unwrap().to_string_lossy().into_owned();
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(&name, &bytes)
        .expect("the image must pass the passive preflight");
    store
}

/// A trace file of this test's own, set before a plug-in is loaded.
fn trace_file(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{tag}.{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    // SAFETY: this test sets the variable for its own process before it loads
    // anything; no other test of this binary reads or writes it.
    unsafe { std::env::set_var("RUSTAMP_HOST_TRACE", &path) };
    path
}

/// `vis_nsfs` answers its entry point with null on any system from major version
/// 6 on. The host asks once, gets null, points the plug-in's version query at
/// Windows XP SP3 and asks again: the module list is the plug-in's own, and the
/// trace names the retry.
#[test]
#[ignore = "VM-only: loads the third-party vis_nsfs.dll in the test VM"]
fn vis_nsfs_is_asked_again_and_loads() {
    require_test_vm();
    require_i686();
    let trace = trace_file("vis-nsfs");
    let dll = staged("vis_nsfs.dll");
    let window = HostWindow::create(IpcSurface::new()).expect("the host window");
    let store = approved(&dll);
    // SAFETY: the store approved this exact image, in the gated test VM, on the
    // UI thread that owns the window.
    let loaded = unsafe { load_plugin_with_store(&dll, &store, &window) }
        .expect("vis_nsfs must load: it is asked again after it refused this system");
    let library = loaded
        .vis_info()
        .expect("a loaded visualization plug-in reports its header and modules");
    assert!(
        !library.modules.is_empty(),
        "the library reports no module: {library:?}"
    );
    let said = std::fs::read_to_string(&trace).unwrap_or_default();
    assert!(
        said.contains("it now hears Windows XP"),
        "the version query was not redirected, so the plug-in's own refusal was not \
         the reason it loaded: {said}"
    );
}

/// The frame a visualization plug-in embeds its window in is drawn from the
/// sheet the player sends: a popup with no system caption, of the sheet's class.
/// `vis_avs` asks for that frame when its first module starts.
#[test]
#[ignore = "VM-only: runs the third-party vis_avs.dll in the test VM"]
fn vis_avs_draws_in_a_frame_from_the_sheet() {
    require_test_vm();
    require_i686();
    // The classic sheet's size, every pixel naming its place: only the frame's
    // kind and style are asserted here, not a skin's picture.
    let (width, height) = (194u32, 109u32);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = index as u32 % width;
        let y = index as u32 / width;
        pixel.copy_from_slice(&[x as u8, y as u8, 0x37, 0xff]);
    }
    embed_frame::set_skin(width, height, rgba);

    let dll = staged("vis_avs.dll");
    let window = HostWindow::create(IpcSurface::new()).expect("the host window");
    let store = approved(&dll);
    // SAFETY: the store approved this exact image, in the gated test VM, on the
    // UI thread that owns the window.
    let mut loaded =
        unsafe { load_plugin_with_store(&dll, &store, &window) }.expect("vis_avs must load");
    loaded
        .vis_info()
        .expect("vis_avs reports its header and modules");
    loaded
        .vis_start(0)
        .expect("AVS starts in the frame it asks the host for");

    // The frame carries the class the ABI fixes for embedded windows.
    let class: Vec<u16> = CLASS_GEN.iter().map(|byte| u16::from(*byte)).collect();
    // SAFETY: a terminated class name; the window belongs to this process and
    // thread, which is the one that started the module.
    let frame = unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) }
        .expect("AVS asked for no frame: the host created none");
    // SAFETY: the frame is ours and alive for as long as the module runs.
    let style = unsafe { GetWindowLongW(frame, GWL_STYLE) } as u32;
    assert_eq!(
        style & WS_POPUP.0,
        WS_POPUP.0,
        "the frame does not draw itself from the sheet: it carries a system frame"
    );
    assert_eq!(
        style & WS_CAPTION.0,
        0,
        "a frame drawn from the sheet has no system caption"
    );

    // AVS destroys its frame when its module quits; one it left is taken down here.
    drop(loaded);
    // SAFETY: our own process; the handle may already be gone.
    if unsafe { IsWindow(Some(frame)) }.as_bool() {
        // SAFETY: the frame is ours, and no module uses it any more.
        let _ = unsafe { DestroyWindow(frame) };
    }
}
