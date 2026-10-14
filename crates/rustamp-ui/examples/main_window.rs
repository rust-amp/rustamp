//! Opens the classic main window and paints one state.
//!
//! ```text
//! cargo run -p rustamp-ui --example main_window [skin-path]
//! ```
//!
//! With no argument the example paints the built-in placeholder skin (flat
//! magenta sheets), so it runs anywhere. Pass a skin folder or `.wsz`/`.zip` to
//! see real artwork, for example the base skin from a webamp checkout.
//!
//! This is a **manual** run: it opens a window and stays open until it is
//! closed. It is never invoked by `cargo test`.

use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_ui::{MainWindowPresenter, MainWindowState, MonoStereo, TransportState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let skin = match std::env::args().nth(1) {
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
        ..MainWindowState::default()
    };

    println!(
        "rustamp-ui main_window: painting {} (close the window to exit)",
        skin.source_description()
    );
    MainWindowPresenter::new(skin, state).run()?;
    Ok(())
}
