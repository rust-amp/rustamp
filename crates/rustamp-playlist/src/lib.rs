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
//! * **Imports** a playlist and reports the format it was detected as
//!   ([`import_playlist_file`], [`import_playlist_path`]), so a caller that
//!   writes the playlist back can use the format it came in.
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
//! * **Interacts**: the playlist editor's model gestures ([`interact`]) — a
//!   single-click selection, a contiguous range selection, a ctrl-click toggle,
//!   a block reorder and the `Delete`/`Ctrl+Delete` removals — as plain
//!   functions over the core
//!   [`Playlist`](rustamp_core::playlist::Playlist), handling current-entry
//!   index shifting through the core's stable ids. The playlist *window* itself
//!   (spec S06 chrome, hit-testing, scrolling, menus) stays with `rustamp-ui`.
//!
//! Everything is written from scratch: the Windows-1252 table
//! ([`cp1252`]), the XML reader ([`xml`]) and the PRNG ([`rng`]). The crate has
//! no dependency other than `rustamp-core`.
//!
//! ## Deliberately out of scope
//!
//! * **Shuffle as a mode** (spec S15): Traversal order and playback state
//!   are managed by the audio engine (`rustamp-audio`); this crate provides
//!   one-shot reordering helpers ([`randomize`], [`shuffle`]).
//! * **Title formatting** `%1`–`%7` (spec S14): titles are taken from the file
//!   verbatim.
//! * **The playlist editor window** (spec S06): the [`interact`] selection,
//!   reorder and delete helpers are here, but scrolling, hit-testing, the bottom
//!   bar, menus and dialogs are the window half in `rustamp-ui`/`rustamp-app`.
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
//!   `<location>`, a dangling `#EXTINF`). The legacy [`read_playlist`] returns
//!   only the surviving entries; [`read_playlist_report`] (and
//!   [`load_playlist_file_report`]) returns a [`PlaylistReport`] providing both
//!   the surviving entries and a list of [`SkippedEntry`] items with reasons.
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
pub mod interact;
pub mod io;
pub mod location;
pub mod m3u;
pub mod order;
pub mod pls;
pub mod rng;
pub mod time;
pub mod xml;
pub mod xspf;

pub use entry::{
    LoadedEntry, PlaylistReport, SkippedEntry, append_loaded, insert_loaded, loaded_entries,
    normalize_title,
};
pub use error::{PlaylistError, Result};
pub use format::{PlaylistFormat, detect, looks_like_text, sniff};
pub use interact::{
    click_select, crop_to_selected, move_selected, move_selected_down, move_selected_up, range_ids,
    remove_selected, select_range, toggle_select_at,
};
pub use io::{
    ImportedPlaylist, import_playlist_file, import_playlist_path, load_playlist_file,
    load_playlist_file_report, read_playlist, read_playlist_as, read_playlist_path,
    read_playlist_report, read_playlist_report_as, read_playlist_report_path,
    read_playlist_report_with_hint, read_playlist_with_hint, write_playlist,
};
pub use location::{Base, resolve_location};
pub use order::{randomize, randomize_with, shuffle, shuffled_ids};
pub use rng::Rng;
