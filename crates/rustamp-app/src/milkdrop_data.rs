// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Tells the user when MilkDrop's data is missing (S21 addendum 1, section 6).
//!
//! Rustamp ships no MilkDrop files. A `vis_milk2.dll` the user added looks for
//! `Milkdrop2\data` and `Milkdrop2\presets` below the plug-ins folder the host
//! answers with: its own folder when that is named `Plugins`, else the host's
//! default one (`rustamp-plughost/src/host_services.rs`).

use std::path::{Path, PathBuf};

/// The files of the data folder (S21 addendum 1, section 6.1).
const DATA_FILES: [&str; 8] = [
    "include.fx",
    "warp_vs.fx",
    "warp_ps.fx",
    "comp_vs.fx",
    "comp_ps.fx",
    "blur_vs.fx",
    "blur1_ps.fx",
    "blur2_ps.fx",
];

/// The note for a plug-in that is about to load, when it is MilkDrop and its
/// data or its presets are missing.
pub(crate) fn missing_data_note(dll: &Path) -> Option<String> {
    let settings = std::env::var_os("RUSTAMP_PLUGIN_SETTINGS").map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    note_for(dll, settings.as_deref(), appdata.as_deref())
}

fn note_for(dll: &Path, settings: Option<&Path>, appdata: Option<&Path>) -> Option<String> {
    let name = dll.file_name()?.to_str()?;
    if !name.eq_ignore_ascii_case("vis_milk2.dll") {
        return None;
    }
    let milkdrop = plugins_folder(dll, settings, appdata)?.join("Milkdrop2");
    let data = milkdrop.join("data");
    if DATA_FILES.iter().any(|file| !data.join(file).is_file()) {
        return Some(format!(
            "error: MilkDrop will not run: its data files are missing from {} (see QUICKSTART.md)",
            data.display()
        ));
    }
    let presets = milkdrop.join("presets");
    let has_preset = std::fs::read_dir(&presets).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("milk"))
        })
    });
    (!has_preset).then(|| {
        format!(
            "error: MilkDrop has no presets in {} (see QUICKSTART.md)",
            presets.display()
        )
    })
}

/// The folder the host names to the plug-in as its plug-ins folder.
fn plugins_folder(dll: &Path, settings: Option<&Path>, appdata: Option<&Path>) -> Option<PathBuf> {
    if let Some(settings) = settings {
        return Some(settings.join("plugins"));
    }
    let parent = dll.parent()?;
    if parent
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("plugins"))
    {
        return Some(parent.to_path_buf());
    }
    Some(appdata?.join("Rustamp").join("plugins"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_data(plugins: &Path) {
        let data = plugins.join("Milkdrop2").join("data");
        std::fs::create_dir_all(&data).unwrap();
        for file in DATA_FILES {
            std::fs::write(data.join(file), b"fixture").unwrap();
        }
    }

    fn a_preset(plugins: &Path) {
        let presets = plugins.join("Milkdrop2").join("presets");
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::write(presets.join("a.MILK"), b"fixture").unwrap();
    }

    #[test]
    fn another_plugin_gets_no_note() {
        assert_eq!(
            note_for(Path::new(r"C:\x\Plugins\vis_avs.dll"), None, None),
            None
        );
    }

    #[test]
    fn missing_data_then_missing_presets_then_nothing() {
        let root = tempfile::tempdir().unwrap();
        let plugins = root.path().join("Plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let dll = plugins.join("VIS_MILK2.dll");

        let note = note_for(&dll, None, None).expect("no data at all");
        assert!(note.contains("data files"), "{note}");
        assert!(note.contains(&plugins.display().to_string()), "{note}");

        full_data(&plugins);
        std::fs::remove_file(plugins.join("Milkdrop2/data/blur2_ps.fx")).unwrap();
        let note = note_for(&dll, None, None).expect("one data file is missing");
        assert!(note.contains("data files"), "{note}");

        full_data(&plugins);
        let note = note_for(&dll, None, None).expect("no presets");
        assert!(note.contains("presets"), "{note}");

        a_preset(&plugins);
        assert_eq!(note_for(&dll, None, None), None);
    }

    #[test]
    fn the_folder_follows_what_the_host_answers() {
        // Paths of the platform the test runs on: a `C:\` path has no parent on Unix.
        let root = Path::new(if cfg!(windows) { r"C:\" } else { "/" });
        let classic_folder = root.join("player").join("plugins");
        let classic = classic_folder.join("vis_milk2.dll");
        let classic = classic.as_path();
        let other = root.join("elsewhere").join("vis_milk2.dll");
        let other = other.as_path();
        let appdata = root.join("appdata");
        let appdata = appdata.as_path();
        let s = root.join("s");
        assert_eq!(
            plugins_folder(classic, None, Some(appdata)),
            Some(classic_folder)
        );
        assert_eq!(
            plugins_folder(other, None, Some(appdata)),
            Some(appdata.join("Rustamp").join("plugins"))
        );
        assert_eq!(
            plugins_folder(classic, Some(s.as_path()), Some(appdata)),
            Some(s.join("plugins"))
        );
    }
}
