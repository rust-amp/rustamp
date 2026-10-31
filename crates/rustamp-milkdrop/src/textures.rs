// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The textures presets name (`sampler_clouds2`): image files in the preset
//! folder, found by their name without the extension.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lactea::{Image, TextureProvider};
use walkdir::WalkDir;

/// The depth the preset folder is searched to, for its images as for its presets.
const MAX_DEPTH: usize = 16;

/// The formats decoded here; the others of [`lactea::TEXTURE_EXTENSIONS`] are
/// left to report as unresolved.
const DECODED: [&str; 5] = ["jpg", "jpeg", "png", "bmp", "tga"];

/// The images of one folder.
pub(crate) struct FolderTextures {
    files: BTreeMap<String, PathBuf>,
}

impl FolderTextures {
    /// Indexes the images of `folder`. The first path in sorted order wins when
    /// two have the same name.
    pub(crate) fn scan(folder: &Path) -> Self {
        let mut paths: Vec<PathBuf> = WalkDir::new(folder)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(walkdir::DirEntry::into_path)
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        DECODED
                            .iter()
                            .any(|decoded| extension.eq_ignore_ascii_case(decoded))
                    })
            })
            .collect();
        paths.sort();
        let mut files = BTreeMap::new();
        for path in paths {
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                files.entry(stem.to_ascii_lowercase()).or_insert(path);
            }
        }
        Self { files }
    }
}

impl TextureProvider for FolderTextures {
    fn load(&mut self, name: &str) -> Option<Image> {
        let path = self.files.get(&name.to_ascii_lowercase())?;
        let decoded = image::open(path).ok()?.into_rgba8();
        let (width, height) = decoded.dimensions();
        Some(Image {
            width,
            height,
            rgba8: decoded.into_raw(),
        })
    }

    fn random_pool(&self) -> Vec<String> {
        // A `BTreeMap` iterates its keys sorted, which the engine's contract asks for.
        self.files.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255])
            .unwrap();
    }

    #[test]
    fn a_texture_is_found_by_its_lowercase_name_in_any_subfolder_and_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let textures = dir.path().join("textures");
        std::fs::create_dir(&textures).unwrap();
        write_png(&textures.join("Clouds2.PNG"));
        let mut provider = FolderTextures::scan(dir.path());
        let image = provider.load("clouds2").expect("the texture is found");
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.rgba8, [255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(provider.random_pool(), ["clouds2"]);
    }

    #[test]
    fn an_unknown_name_and_a_file_that_is_no_image_give_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.png"), b"not an image").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let mut provider = FolderTextures::scan(dir.path());
        assert!(provider.load("missing").is_none());
        assert!(provider.load("broken").is_none());
        assert!(provider.load("notes").is_none());
    }
}
