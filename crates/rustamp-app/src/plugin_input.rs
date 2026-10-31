// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! A classic input plug-in as a decoder factory of the player (see
//! `rustamp_plughost::input_resident`).

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use rustamp_audio::resident::{ResidentConfig, ResidentFactory};
use rustamp_core::audio::DecoderFactory;
use rustamp_plughost::input_resident::{extensions_args, parse_extensions, resident_args};

/// Bounds the question what a plug-in plays: a plug-in's `init` runs for it.
const ASK_WAIT: Duration = Duration::from_secs(10);

/// What `dll` plays, asked of the helper: the 64-bit player cannot map it.
fn extensions_of(helper: &Path, dll: &Path, approvals: &Path) -> Result<Vec<String>, String> {
    let mut child = Command::new(helper)
        .args(extensions_args(dll, approvals))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|error| format!("{}: {error}", helper.display()))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut out = String::new();
        let mut err = String::new();
        let _ = stdout.read_to_string(&mut out);
        let _ = stderr.read_to_string(&mut err);
        let _ = done.send((out, err));
    });
    let Ok((out, err)) = finished.recv_timeout(ASK_WAIT) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("it did not answer in time".to_owned());
    };
    let status = child
        .wait()
        .map_err(|error| format!("{}: {error}", helper.display()))?;
    if !status.success() {
        return Err(err.trim().to_owned());
    }
    Ok(parse_extensions(&out))
}

/// The factory that plays what `dll` claims through a resident per track.
pub fn factory(
    helper: PathBuf,
    dll: &Path,
    approvals: &Path,
) -> Result<Arc<dyn DecoderFactory>, String> {
    let extensions = extensions_of(&helper, dll, approvals)
        .map_err(|problem| format!("{} cannot play: {problem}", dll.display()))?;
    if extensions.is_empty() {
        return Err(format!("{} claims no file type", dll.display()));
    }
    // The registry's factories live as long as the player, and the trait hands
    // out `&str`s; a plug-in is switched on a handful of times per run at most.
    let extensions = extensions
        .into_iter()
        .map(|extension| &*Box::leak(extension.into_boxed_str()))
        .collect();
    let config = ResidentConfig::new(helper)
        .with_helper_args(resident_args(dll, approvals))
        .with_extensions(extensions);
    Ok(Arc::new(ResidentFactory::new(config)))
}
