//! The DSP resident: a classic `dsp_` plug-in in the player's audio path.
//!
//! The engine processes its blocks on its decode thread, and a classic DSP
//! plug-in is a 32-bit DLL, so a block has to cross into a 32-bit process and
//! back. The media-host framework already does that for a resident
//! ([`rustamp_mediahost::dsp`]): a configure handshake, two shared-memory rings
//! and block pumping. This module is the resident behind it, started as
//! `rustamp-plughost32 --dsp-resident`, one process per playing chain.
//!
//! The main thread is the plug-in's UI thread (S09 section 7.2): it owns the
//! host window, loads the plug-in and keeps pumping messages. The framework's
//! serve loop runs on a second thread, which calls `process` like the decode
//! thread of an input plug-in does (S09 section 3.6).
//!
//! A plug-in may answer with fewer or more frames than it got (S09 section 6.2)
//! and the engine's blocks keep their length, so the output passes through a
//! queue. Own choices: blocks of 576 frames, the classic visualization block; a
//! short answer plays silence rather than stopping; and a plug-in that keeps
//! producing more than it gets loses its oldest output past ten seconds.

use std::path::PathBuf;

use rustamp_proto::host::DspStageDesc;

/// The command-line switch that selects this mode of the host binary.
pub const MODE_ARG: &str = "--dsp-resident";

/// The identifier of the one stage this resident serves.
pub const STAGE_ID: &str = "classic-dsp";

/// The stage that names `dll`, approved by the store at `approvals`.
pub fn stage(dll: &std::path::Path, approvals: &std::path::Path) -> DspStageDesc {
    DspStageDesc {
        id: STAGE_ID.to_owned(),
        params: format!("{}\n{}", dll.display(), approvals.display()).into_bytes(),
    }
}

fn parse_stage(params: &[u8]) -> Option<(PathBuf, PathBuf)> {
    let text = std::str::from_utf8(params).ok()?;
    let (dll, approvals) = text.split_once('\n')?;
    Some((PathBuf::from(dll), PathBuf::from(approvals)))
}

#[cfg(windows)]
pub use imp::run;

#[cfg(windows)]
mod imp {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};

    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::dsp::{DspResident, ResidentChain, serve_dsp};
    use rustamp_mediahost::pcm::NamedPcmRings;
    use rustamp_mediahost::{MediaHostError, Result};
    use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

    use super::{STAGE_ID, parse_stage};
    use crate::approval::ApprovalStore;
    use crate::host_window::HostWindow;
    use crate::ipc::IpcSurface;
    use crate::loader::{LoadedPlugin, load_plugin_with_store};

    /// Frames handed to the plug-in at once.
    const BLOCK_FRAMES: usize = 576;
    /// Seconds of surplus output the queue keeps.
    const MAX_QUEUE_SECONDS: usize = 10;

    /// The loaded plug-in, shared with the serve thread.
    ///
    /// The main thread owns it for the whole process and drops it only after
    /// the serve thread has ended. `process` running on another thread than
    /// `init` is the classic contract (S09 section 3.6).
    #[derive(Clone, Copy)]
    struct SharedPlugin(*const LoadedPlugin);
    // SAFETY: see the type's documentation.
    unsafe impl Send for SharedPlugin {}

    /// A load the serve thread asks the main thread for.
    struct LoadRequest {
        dll: PathBuf,
        approvals: PathBuf,
        reply: Sender<std::result::Result<SharedPlugin, String>>,
    }

    struct ClassicResident {
        loads: Sender<LoadRequest>,
    }

    impl DspResident for ClassicResident {
        fn open(
            &self,
            spec: PcmSpecWire,
            stages: &[DspStageDesc],
        ) -> Result<Box<dyn ResidentChain>> {
            let [stage] = stages else {
                return Err(MediaHostError::OperationFailed(format!(
                    "the classic DSP resident serves one stage, not {}",
                    stages.len()
                )));
            };
            if stage.id != STAGE_ID {
                return Err(MediaHostError::OperationFailed(format!(
                    "unknown stage {:?}",
                    stage.id
                )));
            }
            let (dll, approvals) = parse_stage(&stage.params).ok_or_else(|| {
                MediaHostError::OperationFailed("undecodable classic DSP stage".to_owned())
            })?;
            let (reply, answer) = channel();
            self.loads
                .send(LoadRequest {
                    dll,
                    approvals,
                    reply,
                })
                .map_err(|_| {
                    MediaHostError::OperationFailed("the host window is gone".to_owned())
                })?;
            let plugin = answer
                .recv()
                .map_err(|_| MediaHostError::OperationFailed("the host window is gone".to_owned()))?
                .map_err(MediaHostError::OperationFailed)?;
            Ok(Box::new(ClassicChain {
                plugin,
                channels: usize::from(spec.channels),
                sample_rate: spec.sample_rate_hz,
                block: Vec::with_capacity(BLOCK_FRAMES * 2 * usize::from(spec.channels)),
                queue: VecDeque::new(),
            }))
        }
    }

    struct ClassicChain {
        plugin: SharedPlugin,
        channels: usize,
        sample_rate: u32,
        block: Vec<i16>,
        queue: VecDeque<f32>,
    }

    impl ResidentChain for ClassicChain {
        fn latency_frames(&self) -> u32 {
            0
        }

        fn process(&mut self, buf: &mut [f32]) -> Result<()> {
            // SAFETY: the main thread keeps the plug-in alive until this thread
            // has ended (see `SharedPlugin`).
            let plugin = unsafe { &*self.plugin.0 };
            for input in buf.chunks(BLOCK_FRAMES * self.channels) {
                self.block.clear();
                self.block.extend(
                    input
                        .iter()
                        .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16),
                );
                plugin
                    .dsp_process_i16(&mut self.block, self.channels, self.sample_rate)
                    .ok_or_else(|| {
                        MediaHostError::OperationFailed("the DSP plug-in is not running".to_owned())
                    })?;
                self.queue
                    .extend(self.block.iter().map(|sample| f32::from(*sample) / 32767.0));
            }
            let limit = MAX_QUEUE_SECONDS * self.sample_rate as usize * self.channels;
            if self.queue.len() > limit + buf.len() {
                let surplus = self.queue.len() - limit - buf.len();
                self.queue.drain(..surplus);
            }
            let wanted = buf.len().min(self.queue.len());
            let silence = buf.len() - wanted;
            buf[..silence].fill(0.0);
            for (out, sample) in buf[silence..].iter_mut().zip(self.queue.drain(..wanted)) {
                *out = sample;
            }
            Ok(())
        }

        fn reset(&mut self) {
            self.queue.clear();
        }
    }

    /// Loads the plug-in once: a reconfigure asks again and gets the same one.
    fn answer_load(
        request: LoadRequest,
        window: &HostWindow,
        loaded: &mut Option<Box<LoadedPlugin>>,
    ) {
        if loaded.is_none() {
            let store = match ApprovalStore::open_or_empty(&request.approvals) {
                Ok(store) => store,
                Err(error) => {
                    let _ = request.reply.send(Err(format!(
                        "{} could not be read: {error}",
                        request.approvals.display()
                    )));
                    return;
                }
            };
            // SAFETY: the store approved this exact image; a fault in it ends
            // this process, not the player, which is what the process is for.
            match unsafe { load_plugin_with_store(&request.dll, &store, window) } {
                Ok(plugin) => *loaded = Some(Box::new(plugin)),
                Err(error) => {
                    let _ = request.reply.send(Err(format!(
                        "{} could not be loaded: {error:?}",
                        request.dll.display()
                    )));
                    return;
                }
            }
        }
        let plugin = loaded.as_deref().expect("loaded above");
        let _ = request
            .reply
            .send(Ok(SharedPlugin(std::ptr::from_ref(plugin))));
    }

    fn pump_until_done(window: &HostWindow, loads: &Receiver<LoadRequest>, done: &AtomicBool) {
        let mut loaded = None;
        while !done.load(Ordering::Acquire) {
            while let Ok(request) = loads.try_recv() {
                answer_load(request, window, &mut loaded);
            }
            if !crate::message_loop::pump_bounded(window, 64) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // The plug-in's `quit` runs on its UI thread, after the last block.
        drop(loaded);
    }

    /// Runs the resident until the player closes the control channel.
    pub fn run() -> i32 {
        let _fault_guard = crate::fault::install_fault_guard();
        let window = match HostWindow::create(IpcSurface::new()) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("rustamp-plughost32: cannot create host window: {error}");
                return 1;
            }
        };
        let (loads, requests) = channel();
        let done = Arc::new(AtomicBool::new(false));
        let serve = {
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                let result = ControlPeer::start(
                    std::io::stdin(),
                    std::io::stdout(),
                    Liveness::default(),
                    true,
                )
                .and_then(|peer| serve_dsp(&peer, &NamedPcmRings, &ClassicResident { loads }));
                done.store(true, Ordering::Release);
                result
            })
        };
        pump_until_done(&window, &requests, &done);
        match serve.join() {
            Ok(Ok(())) => 0,
            Ok(Err(error)) => {
                eprintln!("rustamp-plughost32: the DSP resident ended: {error}");
                1
            }
            Err(_) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_carries_both_paths() {
        let desc = stage(
            std::path::Path::new(r"C:\Plugins\dsp ä.dll"),
            std::path::Path::new(r"C:\cfg\approved.toml"),
        );
        assert_eq!(desc.id, STAGE_ID);
        assert_eq!(
            parse_stage(&desc.params),
            Some((
                PathBuf::from(r"C:\Plugins\dsp ä.dll"),
                PathBuf::from(r"C:\cfg\approved.toml")
            ))
        );
        assert_eq!(parse_stage(b"no separator"), None);
        assert_eq!(parse_stage(&[0xFF, b'\n']), None);
    }
}
