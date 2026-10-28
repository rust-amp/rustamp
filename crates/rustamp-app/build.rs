// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Embeds the Windows application manifest and the executable's resources.
//!
//! The manifest asks for Common Controls version 6, which is what gives the
//! preferences dialog (tree view, buttons, list box) and the standard file
//! dialog the current Windows look instead of the 1995 one. It reaches the
//! linker directly (`embed-manifest` passes `/MANIFEST`), so the resource
//! section added below does not conflict with it.
//!
//! The resources are the executable's **icon** and its **VERSIONINFO** block,
//! through `winresource`. Without them Explorer, the task bar and the file
//! properties of a shipped `rustamp.exe` show a generic program.
//!
//! # The icon is our own art
//!
//! `icon.rs` does not exist: the pixels are drawn here, from three rectangles
//! and a rounded background, at build time into `OUT_DIR/rustamp.ico`. No
//! third-party image, skin sprite or reference asset is read or copied - the
//! shape is the player's own (a dark tile with three rust-coloured level
//! bars). Changing the look means changing the numbers in [`draw_icon`], and
//! the committed tree stays free of binary art.

/// The sizes inside the `.ico`; Windows picks the one it needs.
#[cfg(windows)]
const ICON_SIZES: [usize; 3] = [16, 32, 48];

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest::embed_manifest(embed_manifest::new_manifest("Rustamp"))
            .expect("could not embed the Windows manifest");
        #[cfg(windows)]
        embed_resources();
    }
    println!("cargo:rerun-if-changed=build.rs");
}

/// Writes the icon and compiles icon plus VERSIONINFO into the `rustamp`
/// binary.
#[cfg(windows)]
fn embed_resources() {
    let out_dir = std::path::PathBuf::from(
        std::env::var_os("OUT_DIR").expect("cargo always sets OUT_DIR for a build script"),
    );
    let icon = out_dir.join("rustamp.ico");
    std::fs::write(&icon, icon_file(&ICON_SIZES)).expect("could not write the icon");
    println!("cargo:rerun-if-changed=build.rs");

    let mut resource = winresource::WindowsResource::new();
    resource
        .set("FileDescription", "Rustamp")
        .set("ProductName", "Rustamp")
        .set("CompanyName", "The Rustamp contributors")
        .set("LegalCopyright", "GPL-3.0-or-later")
        .set("OriginalFilename", "rustamp.exe")
        .set("InternalName", "rustamp")
        .set_icon(
            icon.to_str()
                .expect("OUT_DIR is a path the resource compiler can read"),
        );
    // Scoped to the binary: the test and example executables do not need a
    // second copy of the resources.
    resource
        .compile_for("rustamp")
        .expect("could not embed the Windows resources");
}

/// One `.ico` holding `sizes`, each as a 32-bit BGRA bitmap.
#[cfg(windows)]
fn icon_file(sizes: &[usize]) -> Vec<u8> {
    let images: Vec<(usize, Vec<[u8; 4]>)> =
        sizes.iter().map(|&size| (size, draw_icon(size))).collect();

    let mut file = Vec::new();
    file.extend_from_slice(&0u16.to_le_bytes()); // reserved
    file.extend_from_slice(&1u16.to_le_bytes()); // 1 = icon
    file.extend_from_slice(
        &u16::try_from(images.len())
            .expect("a handful of sizes")
            .to_le_bytes(),
    );

    let mut offset = 6 + 16 * images.len();
    let mut bodies = Vec::with_capacity(images.len());
    for (size, pixels) in &images {
        let body = bitmap(*size, pixels);
        // 0 means 256 in an icon directory entry.
        let edge = u8::try_from(*size).unwrap_or(0);
        file.push(edge);
        file.push(edge);
        file.push(0); // colour count
        file.push(0); // reserved
        file.extend_from_slice(&1u16.to_le_bytes()); // planes
        file.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        file.extend_from_slice(
            &u32::try_from(body.len())
                .expect("a small icon")
                .to_le_bytes(),
        );
        file.extend_from_slice(&u32::try_from(offset).expect("a small icon").to_le_bytes());
        offset += body.len();
        bodies.push(body);
    }
    for body in bodies {
        file.extend_from_slice(&body);
    }
    file
}

/// The icon bitmap: a `BITMAPINFOHEADER`, the BGRA rows bottom-up, then the
/// (all-zero) AND mask a 32-bit icon is expected to carry.
#[cfg(windows)]
fn bitmap(size: usize, pixels: &[[u8; 4]]) -> Vec<u8> {
    let mask_stride = size.div_ceil(32) * 4;
    let mut body = Vec::with_capacity(40 + size * size * 4 + mask_stride * size);
    body.extend_from_slice(&40u32.to_le_bytes()); // header size
    body.extend_from_slice(&i32::try_from(size).expect("a small icon").to_le_bytes());
    // The header height covers the XOR image and the AND mask.
    body.extend_from_slice(&i32::try_from(size * 2).expect("a small icon").to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes()); // planes
    body.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    body.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    body.extend_from_slice(
        &u32::try_from(size * size * 4)
            .expect("a small icon")
            .to_le_bytes(),
    );
    body.extend_from_slice(&[0u8; 16]); // resolution and palette fields
    for y in (0..size).rev() {
        for x in 0..size {
            let [red, green, blue, alpha] = pixels[y * size + x];
            body.extend_from_slice(&[blue, green, red, alpha]);
        }
    }
    body.extend(std::iter::repeat_n(0u8, mask_stride * size));
    body
}

/// Draws the icon at `size` pixels, top-down, as RGBA.
///
/// A dark rounded tile with three level bars of rising and falling height in
/// the player's rust colour. Deliberately hard-edged: the small sizes are the
/// ones that matter and anti-aliasing them by hand would only smear them.
#[cfg(windows)]
fn draw_icon(size: usize) -> Vec<[u8; 4]> {
    const BACKDROP: [u8; 4] = [24, 26, 30, 255];
    const BAR: [u8; 4] = [200, 92, 42, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];

    let scale = size as f32;
    let radius = scale * 0.18;
    // Three bars, centred, sharing the tile's width: 14% wide each with an 8%
    // gap, heights at 34%, 58% and 44% of the tile, 20% above the bottom.
    let bar_width = scale * 0.14;
    let gap = scale * 0.08;
    let group = bar_width * 3.0 + gap * 2.0;
    let left = (scale - group) / 2.0;
    let heights = [0.34, 0.58, 0.44];
    let bottom = scale * 0.80;

    let mut pixels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !inside_rounded_square(px, py, scale, radius) {
                pixels.push(CLEAR);
                continue;
            }
            let mut colour = BACKDROP;
            for (index, height) in heights.iter().enumerate() {
                let bar_left = left + index as f32 * (bar_width + gap);
                let bar_top = bottom - scale * height;
                if (bar_left..bar_left + bar_width).contains(&px) && (bar_top..bottom).contains(&py)
                {
                    colour = BAR;
                }
            }
            pixels.push(colour);
        }
    }
    pixels
}

/// Whether the point lies inside a square of side `size` whose corners are
/// rounded with `radius`.
#[cfg(windows)]
fn inside_rounded_square(x: f32, y: f32, size: f32, radius: f32) -> bool {
    let nearest_x = x.clamp(radius, size - radius);
    let nearest_y = y.clamp(radius, size - radius);
    let dx = x - nearest_x;
    let dy = y - nearest_y;
    dx * dx + dy * dy <= radius * radius
}
