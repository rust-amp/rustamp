//! Audit tests (task card E2): every unsupported or malformed *file* fails
//! loudly, every documented per-entry degradation is pinned, and the losses of
//! a write -> read round trip are stated as assertions.

use std::path::PathBuf;

use rustamp_core::playlist::MediaLocation;
use rustamp_playlist::{
    LoadedEntry, PlaylistError, PlaylistFormat, read_playlist, read_playlist_as,
    read_playlist_path, write_playlist, xspf,
};

fn base() -> PathBuf {
    PathBuf::from("C:/Music/Lists")
}

fn at(name: &str) -> PathBuf {
    base().join(name)
}

fn file(name: &str) -> MediaLocation {
    MediaLocation::File(PathBuf::from(name))
}

fn is_unsupported<T: std::fmt::Debug>(result: Result<T, PlaylistError>) -> bool {
    matches!(result, Err(PlaylistError::UnsupportedFormat(_)))
}

// ---------------------------------------------------------------------------
// 1. Unknown or contradicting input is an error
// ---------------------------------------------------------------------------

#[test]
fn unknown_extension_without_a_marker_is_rejected() {
    let text = b"a.mp3\r\nb.mp3\r\n";
    assert!(is_unsupported(read_playlist_path(text, &at("list.txt"))));
    assert!(is_unsupported(read_playlist_path(text, &at("list.mp3"))));
    // No extension and no path at all keep the bare-list fallback.
    assert_eq!(
        read_playlist_path(text, &at("list")).unwrap().len(),
        2,
        "no extension: bare list"
    );
    assert_eq!(read_playlist(text, &base()).unwrap().len(), 2);
}

#[test]
fn a_content_marker_overrides_an_unknown_extension() {
    let entries = read_playlist_path(b"#EXTM3U\r\na.mp3\r\n", &at("list.txt")).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn binary_data_is_rejected_whatever_the_extension() {
    let blob = [0x89u8, b'P', b'N', b'G', 0x00, 0x01, 0x02, 0x03];
    for name in ["x.m3u", "x.m3u8", "x.pls", "x.xspf", "x.b4s", "x"] {
        assert!(
            is_unsupported(read_playlist_path(&blob, &at(name))),
            "{name}"
        );
    }
}

#[test]
fn utf16_playlists_are_rejected_not_read_as_junk() {
    // "#EXTM3U\r\na.mp3\r\n" as UTF-16LE with a BOM; the crate does not
    // decode UTF-16 (CONTRACT_NOTES.md section 3).
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "#EXTM3U\r\na.mp3\r\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    assert!(is_unsupported(read_playlist_path(&bytes, &at("x.m3u"))));
    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::M3u,
        &bytes,
        &base()
    )));
}

#[test]
fn xml_with_another_root_is_rejected() {
    let html = b"<html><body>not a playlist</body></html>";
    for name in ["x.m3u", "x.pls", "x.xspf"] {
        assert!(
            is_unsupported(read_playlist_path(html, &at(name))),
            "{name}"
        );
    }
    for format in [
        PlaylistFormat::M3u,
        PlaylistFormat::Pls,
        PlaylistFormat::Xspf,
    ] {
        assert!(is_unsupported(read_playlist_as(format, html, &base())));
    }
}

#[test]
fn text_behind_an_xml_extension_is_rejected() {
    for name in ["x.xspf", "x.b4s"] {
        assert!(
            is_unsupported(read_playlist_path(b"a.mp3\r\n", &at(name))),
            "{name}"
        );
    }
}

#[test]
fn a_misnamed_pls_still_reads_as_a_bare_m3u_list() {
    // Documented degradation, kept from the first card: marker-less text is
    // an M3U list whatever `.pls`/`.m3u` says.
    let entries = read_playlist_path(b"a.mp3\r\n", &at("x.pls")).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn an_explicit_format_that_the_content_contradicts_is_rejected() {
    let m3u = b"#EXTM3U\r\na.mp3\r\n";
    let pls = b"[playlist]\r\nFile1=a.mp3\r\n";
    let xspf_doc = b"<playlist><track><location>a.mp3</location></track></playlist>";

    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::Pls,
        m3u,
        &base()
    )));
    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::Xspf,
        m3u,
        &base()
    )));
    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::M3u,
        pls,
        &base()
    )));
    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::Xspf,
        pls,
        &base()
    )));
    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::M3u,
        xspf_doc,
        &base()
    )));
    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::Pls,
        xspf_doc,
        &base()
    )));

    // The matching or compatible combinations still work.
    assert_eq!(
        read_playlist_as(PlaylistFormat::M3u, m3u, &base())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        read_playlist_as(PlaylistFormat::M3u8, m3u, &base())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        read_playlist_as(PlaylistFormat::Pls, pls, &base())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        read_playlist_as(PlaylistFormat::Xspf, xspf_doc, &base())
            .unwrap()
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// 2. Truncated XSPF
// ---------------------------------------------------------------------------

#[test]
fn a_truncated_xspf_is_an_error_at_every_entry_point() {
    let cut = b"<playlist><trackList><track><location>a.mp3</location>";
    assert!(is_unsupported(read_playlist(cut, &base())));
    assert!(is_unsupported(read_playlist_path(cut, &at("x.xspf"))));
    assert!(is_unsupported(read_playlist_as(
        PlaylistFormat::Xspf,
        cut,
        &base()
    )));
    // The module-level reader keeps its documented leniency.
    assert_eq!(xspf::read(cut, &base()).len(), 1);
}

#[test]
fn xspf_validation_names_the_defect() {
    let why = |text: &str| xspf::validate(text).unwrap_err();
    assert!(why("").contains("no <playlist>"));
    assert!(why("<?xml version=\"1.0\"?>").contains("no <playlist>"));
    assert!(why("<playlist><track>").contains("truncated"));
    assert!(why("<playlist><track></playlist>").contains("closes"));
    assert!(why("<other/>").contains("not <playlist>"));
    assert!(why("<playlist/><playlist/>").contains("more than one root"));
    assert!(xspf::validate("<playlist/>").is_ok());
    assert!(xspf::validate("<X:Playlist xmlns:X=\"u\"><X:track/></x:PLAYLIST>").is_ok());
}

#[test]
fn an_empty_xspf_file_is_not_a_valid_xspf() {
    assert!(is_unsupported(read_playlist_path(b"", &at("x.xspf"))));
    // An empty M3U or PLS is just an empty list.
    assert!(read_playlist_path(b"", &at("x.m3u")).unwrap().is_empty());
    assert!(read_playlist_path(b"", &at("x.pls")).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// 3. B4S is named as unsupported, everywhere
// ---------------------------------------------------------------------------

#[test]
fn b4s_is_unsupported_for_every_way_in() {
    let b4s = b"<?xml version=\"1.0\"?><WinampXML><playlist num_entries=\"0\"/></WinampXML>";
    for result in [
        read_playlist(b4s, &base()),
        read_playlist_path(b4s, &at("x.b4s")),
        read_playlist_as(PlaylistFormat::B4s, b4s, &base()),
        read_playlist_as(PlaylistFormat::B4s, b"", &base()),
    ] {
        let Err(PlaylistError::UnsupportedFormat(message)) = result else {
            panic!("B4S must be an UnsupportedFormat error");
        };
        assert!(message.contains("B4S is not implemented"), "{message}");
    }
    assert!(is_unsupported(write_playlist(PlaylistFormat::B4s, &[])));
    assert!(!PlaylistFormat::B4s.is_supported());
    for format in PlaylistFormat::ALL {
        if format != PlaylistFormat::B4s {
            assert!(format.is_supported(), "{format}");
            assert!(write_playlist(format, &[]).is_ok(), "{format}");
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Per-entry degradation inside a well-formed file (documented, silent)
// ---------------------------------------------------------------------------

#[test]
fn bad_entries_in_a_good_file_are_skipped_and_the_rest_survive() {
    // A dangling #EXTINF with no location after it.
    let m3u = read_playlist(b"#EXTM3U\r\n#EXTINF:5,Orphan\r\n", &base()).unwrap();
    assert!(m3u.is_empty());

    // FileN with an empty value, TitleN without FileN.
    let pls = read_playlist(
        b"[playlist]\r\nFile1=\r\nTitle1=x\r\nTitle2=y\r\nFile3=c.mp3\r\n",
        &base(),
    )
    .unwrap();
    assert_eq!(pls.len(), 1);
    assert_eq!(pls[0].title, None);

    // A track without <location> in a complete document.
    let xspf_doc = b"<playlist><trackList><track><title>t</title></track>\
                     <track><location>a.mp3</location></track></trackList></playlist>";
    assert_eq!(read_playlist(xspf_doc, &base()).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// 5. Writers refuse what they cannot store faithfully
// ---------------------------------------------------------------------------

#[test]
fn writers_reject_locations_that_would_change() {
    let bad: Vec<(&str, LoadedEntry)> = vec![
        ("empty", LoadedEntry::new(file(""))),
        ("padded", LoadedEntry::new(file("C:/Music/a.mp3 "))),
        ("newline", LoadedEntry::new(file("C:/Music/a\nb.mp3"))),
    ];
    for (name, entry) in &bad {
        for format in [
            PlaylistFormat::M3u,
            PlaylistFormat::M3u8,
            PlaylistFormat::Pls,
        ] {
            let result = write_playlist(format, std::slice::from_ref(entry));
            assert!(
                matches!(result, Err(PlaylistError::InvalidEntry(_))),
                "{name} {format}"
            );
        }
    }
    // XSPF escapes line breaks, so only the empty and padded locations fail.
    assert!(matches!(
        write_playlist(PlaylistFormat::Xspf, &[bad[0].1.clone()]),
        Err(PlaylistError::InvalidEntry(_))
    ));
    assert!(matches!(
        write_playlist(PlaylistFormat::Xspf, &[bad[1].1.clone()]),
        Err(PlaylistError::InvalidEntry(_))
    ));
    assert!(write_playlist(PlaylistFormat::Xspf, &[bad[2].1.clone()]).is_ok());
}

#[test]
fn m3u_rejects_a_location_that_would_read_back_as_a_comment() {
    let entry = LoadedEntry::new(file("#1.mp3"));
    for format in [PlaylistFormat::M3u, PlaylistFormat::M3u8] {
        assert!(matches!(
            write_playlist(format, std::slice::from_ref(&entry)),
            Err(PlaylistError::InvalidEntry(_))
        ));
    }
    // PLS and XSPF carry the location after `File1=` / inside an element.
    assert!(write_playlist(PlaylistFormat::Pls, std::slice::from_ref(&entry)).is_ok());
    assert!(write_playlist(PlaylistFormat::Xspf, std::slice::from_ref(&entry)).is_ok());
}

#[test]
fn invalid_entry_maps_to_the_core_invalid_argument_error() {
    let error = PlaylistError::InvalidEntry("x".into());
    assert_eq!(error.to_string(), "invalid playlist entry: x");
    assert!(matches!(
        rustamp_core::Error::from(error),
        rustamp_core::Error::InvalidArgument(_)
    ));
}

// ---------------------------------------------------------------------------
// 6. What a round trip loses (stated, not hidden)
// ---------------------------------------------------------------------------

fn round_trip(format: PlaylistFormat, entry: &LoadedEntry) -> LoadedEntry {
    let bytes = write_playlist(format, std::slice::from_ref(entry)).unwrap();
    let mut back = read_playlist_path(&bytes, &at(&format!("x.{}", format.extension()))).unwrap();
    assert_eq!(back.len(), 1, "{format}");
    back.remove(0)
}

const SUPPORTED: [PlaylistFormat; 4] = [
    PlaylistFormat::M3u,
    PlaylistFormat::M3u8,
    PlaylistFormat::Pls,
    PlaylistFormat::Xspf,
];

#[test]
fn titles_come_back_trimmed_and_empty_titles_become_none() {
    for format in SUPPORTED {
        let padded = LoadedEntry {
            location: file("C:/Music/a.mp3"),
            title: Some("  padded  ".into()),
            duration_ms: Some(1500),
        };
        let back = round_trip(format, &padded);
        assert_eq!(back.title.as_deref(), Some("padded"), "{format}");
        assert_eq!(back.duration_ms, Some(1500), "{format}");

        let empty = LoadedEntry {
            location: file("C:/Music/a.mp3"),
            title: Some(String::new()),
            duration_ms: None,
        };
        assert_eq!(round_trip(format, &empty).title, None, "{format}");
    }
}

#[test]
fn a_line_break_in_a_title_survives_only_in_xspf() {
    let entry = LoadedEntry {
        location: file("C:/Music/a.mp3"),
        title: Some("a\nb".into()),
        duration_ms: None,
    };
    for format in [
        PlaylistFormat::M3u,
        PlaylistFormat::M3u8,
        PlaylistFormat::Pls,
    ] {
        assert_eq!(
            round_trip(format, &entry).title.as_deref(),
            Some("a b"),
            "{format}"
        );
    }
    assert_eq!(
        round_trip(PlaylistFormat::Xspf, &entry).title.as_deref(),
        Some("a\nb")
    );
}

#[test]
fn artist_and_album_from_a_playlist_file_are_dropped() {
    // `LoadedEntry` has no artist/album field (CONTRACT_NOTES.md 2.3); the
    // M3U directives and the XSPF elements are parsed away, not kept.
    let m3u = read_playlist(
        b"#EXTM3U\r\n#EXTART:Artist\r\n#EXTALBUM:Album\r\n#EXTINF:1,Title\r\na.mp3\r\n",
        &base(),
    )
    .unwrap();
    assert_eq!(m3u[0].title.as_deref(), Some("Title"));

    let xspf_doc = b"<playlist><track><location>a.mp3</location><title>Title</title>\
                     <creator>Artist</creator><album>Album</album></track></playlist>";
    let entries = read_playlist(xspf_doc, &base()).unwrap();
    assert_eq!(entries[0].title.as_deref(), Some("Title"));
}
