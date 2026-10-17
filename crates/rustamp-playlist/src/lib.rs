// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-playlist
//!
//! Playlist file I/O and one-shot reordering, on top of the frozen
//! [`rustamp_core::playlist::Playlist`] model.
//!
//! ## What this crate does
//!
//! * **Reads** M3U, M3U8, PLS and XSPF from bytes ([`read_playlist`]) or from
//!   disk ([`load_playlist_file`]), detecting the format from the content and
//!   from the file name extension ([`PlaylistFormat`]).
//! * **Writes** M3U/M3U8, PLS and XSPF ([`write_playlist`]) so that
//!   write → read round-trips order, locations, titles and durations.
//! * **Resolves** relative entries against the playlist file's directory, not
//!   the process working directory, including `file:` and `http(s):` URLs
//!   ([`location`]).
//! * **Reorders**: [`randomize`] applies one random permutation through
//!   [`rustamp_core::playlist::Playlist::apply_order`]. `sort` and `reverse` already
//!   exist in core and are not duplicated here.
//! * **Loads** entries into a [`rustamp_core::playlist::Playlist`] through
//!   `insert_many`/`push` ([`append_loaded`], [`insert_loaded`]), so entry ids
//!   stay stable and are never reused.
//!
//! Everything is written from scratch: the Windows-1252 table
//! ([`cp1252`]), the XML reader ([`xml`]) and the PRNG ([`rng`]). The crate has
//! no dependency other than `rustamp-core`.
//!
//! ## Deliberately out of scope
//!
//! * **Shuffle as a mode** (spec S15): only the one-shot [`randomize`] is here.
//! * **Title formatting** `%1`–`%7` (spec S14): titles are taken from the file
//!   verbatim.
//! * **The playlist editor** (spec S06): no selection, drag, scroll or menu
//!   behaviour, only model-level helpers.
//! * **B4S**: recognized and rejected with a clear error, because it is a
//!   Winamp-specific XML dialect with no published format description available
//!   to this task. See `CONTRACT_NOTES.md`.
//!
//! ## Robustness contract
//!
//! The readers never panic. Two levels, on purpose:
//!
//! * A bad **entry** inside a well-formed file is skipped according to the
//!   rules in each format module (empty `FileN=`, a track without
//!   `<location>`, a dangling `#EXTINF`). This is silent: the entry points
//!   return only the surviving entries, with no count of skipped ones.
//! * A bad **file** is an error from every entry point in [`io`]: unknown or
//!   binary/UTF-16 content, XML with another root, an unknown extension
//!   without a content marker, a file whose content contradicts the format it
//!   is read as, a truncated XSPF document, and B4S.
//!
//! Writers fail with [`PlaylistError::InvalidEntry`] instead of storing an
//! entry in a changed form. See `CONTRACT_NOTES.md` §4 for what a round trip
//! loses.
//!
//! The crate is `#![forbid(unsafe_code)]`.

#![forbid(unsafe_code)]

pub mod cp1252;
pub mod entry;
pub mod error;
pub mod format;
pub mod io;
pub mod location;
pub mod m3u;
pub mod order;
pub mod pls;
pub mod rng;
pub mod time;
pub mod xml;
pub mod xspf;

pub use entry::{LoadedEntry, append_loaded, insert_loaded, loaded_entries, normalize_title};
pub use error::{PlaylistError, Result};
pub use format::{PlaylistFormat, detect, looks_like_text, sniff};
pub use io::{
    load_playlist_file, read_playlist, read_playlist_as, read_playlist_path,
    read_playlist_with_hint, write_playlist,
};
pub use location::{Base, resolve_location};
pub use order::{randomize, randomize_with, shuffle, shuffled_ids};
pub use rng::Rng;
