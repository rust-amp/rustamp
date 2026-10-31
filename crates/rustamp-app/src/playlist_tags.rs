// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The tags of the playlist's rows, read off the UI thread.
//!
//! The playlist's row text may show the tags of a file (spec **S14**'s
//! `%artist%` and the rest), and reading a file is I/O: it must not happen
//! while the player paints a frame. The reader therefore asks a worker
//! thread for the tags and takes the results when it paints.
//!
//! # Two sources, one cache
//!
//! * The **engine** reads the tags of a track when it opens it and reports
//!   them (see [`rustamp_core::event::Event::TrackChanged`]). The app keeps
//!   them ([`TagReader::remember`]): they are in memory already, and nothing
//!   is read for them.
//! * The **worker** reads the tags of the files the playlist shows, so a row
//!   that has not played yet carries its tags as well.
//!
//! # The worker ends with the app
//!
//! The worker waits for the jobs the app sends. The app owns the channel, so
//! the worker ends when the app drops it: nobody waits for the tags of a
//! player that is gone.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use rustamp_core::meta::TrackMetadata;
use rustamp_core::playlist::EntryId;

/// A job: the row whose tags are wanted and the file to read them from.
struct Job {
    /// The row the tags belong to.
    id: EntryId,
    /// The file the tags are read from.
    path: PathBuf,
}

/// The tags of one row.
struct Tags {
    /// The row the tags belong to.
    id: EntryId,
    /// The tags the file carries; empty when the file has none.
    metadata: TrackMetadata,
}

/// The reader the worker uses.
///
/// The seam the tests use: a real read touches the disk, and the tests need
/// a reader they can steer.
type ReadTags = Box<dyn Fn(&Path) -> Option<TrackMetadata> + Send>;

/// The tags of the playlist's rows.
pub struct TagReader {
    /// The channel to the worker; the worker starts with the first request.
    jobs: Option<Sender<Job>>,
    /// The jobs the worker takes; they move into the worker when it starts.
    requests: Option<Receiver<Job>>,
    /// The channel the worker's results come in on.
    outbox: Option<Sender<Tags>>,
    /// The results the worker sent.
    results: Receiver<Tags>,
    /// The tags the app has, per row.
    tags: HashMap<EntryId, TrackMetadata>,
    /// The rows whose tags were already asked for.
    asked: HashSet<EntryId>,
    /// The reader the worker uses; it moves into the worker when it starts.
    read: Option<ReadTags>,
}

impl Default for TagReader {
    fn default() -> Self {
        Self::new()
    }
}

impl TagReader {
    /// A reader that reads the tags of a file from the disk.
    pub fn new() -> Self {
        Self::with_reader(|path| rustamp_meta::read_tags(path).ok())
    }

    /// A reader that also opens the file with `registry` when its tags name no
    /// length, so the row can show one. Opening decodes no audio to speak of,
    /// and it happens on the worker, for the rows the playlist shows.
    pub fn with_length(registry: rustamp_core::audio::DecoderRegistry) -> Self {
        Self::with_reader(move |path| {
            let mut tags = rustamp_meta::read_tags(path).ok().unwrap_or_default();
            if tags.duration_ms.is_none()
                && let Ok(decoder) =
                    registry.open(rustamp_core::audio::MediaSource::Path(path.to_path_buf()))
            {
                tags.duration_ms = decoder.duration().map(|length| length.as_millis() as u64);
            }
            Some(tags)
        })
    }

    /// A reader that asks `read` for the tags of a file.
    pub fn with_reader(read: impl Fn(&Path) -> Option<TrackMetadata> + Send + 'static) -> Self {
        let (jobs, requests) = mpsc::channel();
        let (outbox, results) = mpsc::channel();
        Self {
            jobs: Some(jobs),
            requests: Some(requests),
            outbox: Some(outbox),
            results,
            tags: HashMap::new(),
            asked: HashSet::new(),
            read: Some(Box::new(read)),
        }
    }

    /// Asks for the tags of the row `id`.
    ///
    /// The row is asked for once: a file whose tags are not there is not
    /// asked for again.
    pub fn request(&mut self, id: EntryId, path: &Path) {
        if !self.asked.insert(id) {
            return;
        }
        self.start_worker();
        let Some(jobs) = &self.jobs else {
            return;
        };
        // The app does not wait for the tags: a row shows its plain text
        // until they arrive.
        let _ = jobs.send(Job {
            id,
            path: path.to_path_buf(),
        });
    }

    /// Keeps the metadata the engine reported for the row `id`.
    ///
    /// The engine reads the tags of a track when it opens it, so the tags of
    /// the track that plays are in memory already: they are kept here
    /// without reading anything.
    pub fn remember(&mut self, id: EntryId, metadata: TrackMetadata) {
        self.tags.insert(id, metadata);
    }

    /// Takes the tags that arrived.
    pub fn poll(&mut self) {
        while let Ok(tags) = self.results.try_recv() {
            self.tags.insert(tags.id, tags.metadata);
        }
    }

    /// The tags the app has for the row `id`.
    pub fn metadata(&self, id: EntryId) -> Option<&TrackMetadata> {
        self.tags.get(&id)
    }

    /// Starts the worker, if it is not running yet.
    ///
    /// The worker is started with the first request: a player that never
    /// shows a playlist row never reads a file.
    fn start_worker(&mut self) {
        if self.requests.is_none() {
            return;
        }
        let Some(requests) = self.requests.take() else {
            return;
        };
        let Some(read) = self.read.take() else {
            return;
        };
        let Some(outbox) = self.outbox.take() else {
            return;
        };
        // The worker reads the tags and sends them back. It ends when the
        // app is gone: the app drops the channels, and nobody waits for the
        // tags of a player that is closing.
        thread::spawn(move || {
            while let Ok(job) = requests.recv() {
                let metadata = read(&job.path).unwrap_or_default();
                if outbox
                    .send(Tags {
                        id: job.id,
                        metadata,
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A metadata with a title, for the reader the tests steer.
    fn tags(title: &str) -> TrackMetadata {
        TrackMetadata {
            title: Some(title.to_string()),
            ..TrackMetadata::default()
        }
    }

    #[test]
    fn the_tags_of_a_row_arrive_from_the_worker() {
        let mut reader = TagReader::with_reader(|path| {
            if path.ends_with("song.mp3") {
                Some(tags("The Title"))
            } else {
                None
            }
        });
        let id = EntryId(7);
        reader.request(id, Path::new("C:\\Music\\song.mp3"));
        // The worker reads the file on its own thread: the app waits for the
        // result, but only for as long as a read takes.
        let mut waited = 0;
        while reader.metadata(id).is_none() && waited < 2_000 {
            reader.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
            waited += 1;
        }
        assert_eq!(
            reader.metadata(id).and_then(|meta| meta.title.as_deref()),
            Some("The Title")
        );
    }

    #[test]
    fn a_row_is_asked_for_once() {
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = reads.clone();
        let mut reader = TagReader::with_reader(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(tags("The Title"))
        });
        let id = EntryId(1);
        reader.request(id, Path::new("song.mp3"));
        reader.request(id, Path::new("song.mp3"));
        // The worker reads the file once: the second request is dropped.
        let mut waited = 0;
        while reads.load(std::sync::atomic::Ordering::SeqCst) == 0 && waited < 2_000 {
            std::thread::sleep(std::time::Duration::from_millis(1));
            waited += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn the_metadata_the_engine_reported_is_kept_without_a_read() {
        let mut reader = TagReader::new();
        let id = EntryId(3);
        reader.remember(id, tags("The Title"));
        assert_eq!(
            reader.metadata(id).and_then(|meta| meta.title.as_deref()),
            Some("The Title")
        );
    }
}
