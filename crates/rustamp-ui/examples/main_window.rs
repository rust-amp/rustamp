//! Opens the classic main window and paints one state.
//!
//! ```text
//! cargo run -p rustamp-ui --example main_window [skin-path] [--double-size]
//! ```
//!
//! With no argument the example paints the built-in placeholder skin (flat
//! magenta sheets), so it runs anywhere. Pass a skin folder or `.wsz`/`.zip` to
//! see real artwork, for example the base skin from a webamp checkout.
//! `--double-size` starts the window at 2x (550x232); it exists so the
//! double-size client area can be measured without sending any input.
//!
//! This is a **manual** run: it opens a window and stays open until it is
//! closed. It is never invoked by `cargo test`.

use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_ui::{MainWindowPresenter, MainWindowState, MonoStereo, TransportState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut skin_arg: Option<String> = None;
    let mut double_size = false;
    for arg in std::env::args().skip(1) {
        if arg == "--double-size" {
            double_size = true;
        } else if skin_arg.is_none() {
            skin_arg = Some(arg);
        }
    }
    let skin = match skin_arg {
        Some(path) => Skin::load(&path)?,
        None => {
            let mut source = MemorySource::new("built-in placeholder");
            Skin::from_source(&mut source, &BuiltinDefaultSkin)?
        }
    };

    let state = MainWindowState {
        transport: TransportState::Playing,
        title_bar_focused: true,
        clutter_a: true,
        clutter_d: true,
        shuffle: true,
        repeat: true,
        time_seconds: 3 * 60 + 7,
        title: "Rustamp - main window".to_string(),
        kbps: Some(128),
        khz: Some(44),
        channels: MonoStereo::Stereo,
        position: 42,
        volume: 78,
        balance: 50,
        double_size,
        ..MainWindowState::default()
    };

    println!(
        "rustamp-ui main_window: painting {} (close the window to exit)",
        skin.source_description()
    );
    MainWindowPresenter::from_state(skin, state).run()?;
    Ok(())
}
