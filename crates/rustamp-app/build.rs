// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Embeds the Windows application manifest.
//!
//! The manifest asks for Common Controls version 6, which is what gives the
//! preferences dialog (tree view, buttons, list box) and the standard file
//! dialog the current Windows look instead of the 1995 one.

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest::embed_manifest(embed_manifest::new_manifest("Rustamp"))
            .expect("could not embed the Windows manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
