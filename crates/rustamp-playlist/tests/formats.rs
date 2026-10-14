//! Integration tests for the playlist readers and writers: the acceptance
//! tests of task card E1 that exercise the public API.
//!
//! There is deliberately no `tempfile` dev-dependency; the one test that needs
//! real directories creates and removes them itself.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rustamp_core::playlist::{MediaLocation, Playlist};
use rustamp_playlist::{
    LoadedEntry, PlaylistFormat, append_loaded, load_playlist_file, read_playlist,
    read_playlist_as, read_playlist_path, write_playlist,
};

/// A uniquely named directory under the system temp directory, removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustamp-playlist-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create the temp directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn file(name: &str) -> MediaLocation {
    MediaLocation::File(PathBuf::from(name))
}

fn url(text: &str) -> MediaLocation {
    MediaLocation::Url(text.to_owned())
}

fn base() -> PathBuf {
    PathBuf::from("C:/Music/Lists")
}

/// A small list used by the round-trip tests: every combination of
/// title/duration presence, a local file and a URL.
fn sample_entries() -> Vec<LoadedEntry> {
    vec![
        LoadedEntry::new(file("C:/Music/a.mp3"))
            .with_title("A Song, with a comma")
            .with_duration_ms(123_456),
        LoadedEntry::new(file("C:/Music/sub/b with spaces.mp3")).with_title("B"),
        LoadedEntry::new(file("C:/Music/c.mp3")).with_duration_ms(1000),
        LoadedEntry::new(file("C:/Music/d.mp3")),
        LoadedEntry::new(url("http://example.com/live?x=1&y=2")).with_title("Radio"),
    ]
}

// ---------------------------------------------------------------------------
// small hand-written samples
// ---------------------------------------------------------------------------

#[test]
fn m3u_sample_parses_to_the_expected_entries() {
    let text = "#EXTM3U\r\n\
                #EXTINF:123,A Song\r\n\
                a.mp3\r\n\
                #EXTINF:4.5,Another - One\r\n\
                sub/b.mp3\r\n\
                #EXTALBUM:An Album\r\n\
                #EXTART:An Artist\r\n\
                #EXTINF:-1,Radio\r\n\
                http://example.com/live\r\n\
                plain.mp3\r\n";
    let entries = read_playlist_path(text.as_bytes(), &base().join("list.m3u")).unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].location, file("C:/Music/Lists/a.mp3"));
    assert_eq!(entries[0].title.as_deref(), Some("A Song"));
    assert_eq!(entries[0].duration_ms, Some(123_000));
    assert_eq!(entries[1].location, file("C:/Music/Lists/sub/b.mp3"));
    assert_eq!(entries[1].title.as_deref(), Some("Another - One"));
    assert_eq!(entries[1].duration_ms, Some(4500));
    assert_eq!(entries[2].location, url("http://example.com/live"));
    assert_eq!(entries[2].duration_ms, None);
    assert_eq!(entries[3].title, None);
}

#[test]
fn m3u8_sample_parses_utf8_with_a_bom() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("#EXTM3U\r\n#EXTINF:5,M\u{00FC}rchen \u{20AC}\r\na.mp3\r\n".as_bytes());
    let entries = read_playlist_path(&bytes, &base().join("list.m3u8")).unwrap();
    assert_eq!(entries[0].title.as_deref(), Some("M\u{00FC}rchen \u{20AC}"));
}

#[test]
fn pls_sample_parses_to_the_expected_entries() {
    let text = "[playlist]\r\n\
                File1=a.mp3\r\nTitle1=First\r\nLength1=123\r\n\
                File2=b.mp3\r\nTitle2=Second\r\nLength2=4.5\r\n\
                File3=c.mp3\r\n\
                NumberOfEntries=3\r\nVersion=2\r\n";
    let entries = read_playlist_path(text.as_bytes(), &base().join("list.pls")).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].location, file("C:/Music/Lists/a.mp3"));
    assert_eq!(entries[0].title.as_deref(), Some("First"));
    assert_eq!(entries[0].duration_ms, Some(123_000));
    assert_eq!(entries[1].duration_ms, Some(4500));
    assert_eq!(entries[2].title, None);
    assert_eq!(entries[2].duration_ms, None);
}

#[test]
fn xspf_sample_parses_to_the_expected_entries() {
    let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<playlist version="1" xmlns="http://xspf.org/ns/0/" xml:base="file:///C:/Music/">
  <title>Sample</title>
  <trackList>
    <track>
      <location>a.mp3</location>
      <title>A &amp; B</title>
      <duration>123456</duration>
    </track>
    <track>
      <location><![CDATA[sub/b.mp3]]></location>
      <title>B</title>
    </track>
    <track>
      <location>http://example.com/live</location>
    </track>
  </trackList>
</playlist>"#;
    let entries = read_playlist_path(text.as_bytes(), &base().join("list.xspf")).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].location, file("C:/Music/a.mp3"));
    assert_eq!(entries[0].title.as_deref(), Some("A & B"));
    assert_eq!(entries[0].duration_ms, Some(123_456));
    assert_eq!(entries[1].location, file("C:/Music/sub/b.mp3"));
    assert_eq!(entries[2].location, url("http://example.com/live"));
}

// ---------------------------------------------------------------------------
// round-trips
// ---------------------------------------------------------------------------

#[test]
fn every_supported_format_round_trips() {
    let entries = sample_entries();
    for format in [
        PlaylistFormat::M3u,
        PlaylistFormat::M3u8,
        PlaylistFormat::Pls,
        PlaylistFormat::Xspf,
    ] {
        let bytes = write_playlist(format, &entries).expect("write");
        let path = base().join(format!("list.{}", format.extension()));
        let back = read_playlist_path(&bytes, &path).expect("read");
        assert_eq!(back, entries, "round-trip failed for {format}");
    }
}

#[test]
fn reading_is_independent_of_the_format_hint_for_unambiguous_content() {
    let entries = sample_entries();
    let bytes = write_playlist(PlaylistFormat::Pls, &entries).unwrap();
    // The `[playlist]` marker wins even with a wrong extension.
    let back = read_playlist_path(&bytes, &base().join("list.m3u")).unwrap();
    assert_eq!(back, entries);
}

// ---------------------------------------------------------------------------
// path resolution
// ---------------------------------------------------------------------------

#[test]
fn relative_paths_resolve_against_the_playlist_directory() {
    let dir = TempDir::new("relative");
    let lists = dir.path().join("lists");
    let sub = lists.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let track = sub.join("track.mp3");
    std::fs::write(&track, b"not really audio").unwrap();

    let text = "#EXTM3U\r\n#EXTINF:1,A\r\nsub/track.mp3\r\nsub\\track.mp3\r\n";
    let list_path = lists.join("list.m3u");
    let entries = read_playlist_path(text.as_bytes(), &list_path).unwrap();

    let MediaLocation::File(resolved) = &entries[0].location else {
        panic!("expected a file location");
    };
    assert!(resolved.is_absolute(), "{resolved:?} must be absolute");
    assert!(
        resolved.exists(),
        "{resolved:?} must be the file we created"
    );
    assert_eq!(resolved, &track);
    // Both separator styles resolve to the same file.
    assert_eq!(entries[0].location, entries[1].location);

    // Proof that the process working directory plays no part: resolving the
    // same bytes against the cwd or against another directory yields different,
    // non-existent paths.
    let cwd = std::env::current_dir().unwrap();
    assert_ne!(resolved, &cwd.join("sub").join("track.mp3"));
    let elsewhere = read_playlist(text.as_bytes(), dir.path()).unwrap();
    assert_ne!(entries[0].location, elsewhere[0].location);
    let MediaLocation::File(other) = &elsewhere[0].location else {
        panic!("expected a file location");
    };
    assert!(!other.exists());
}

#[test]
fn load_playlist_file_uses_the_file_directory() {
    let dir = TempDir::new("load");
    let sub = dir.path().join("playlists");
    std::fs::create_dir_all(&sub).unwrap();
    let track = sub.join("song.mp3");
    std::fs::write(&track, b"x").unwrap();
    let list = sub.join("list.pls");
    std::fs::write(&list, "[playlist]\r\nFile1=song.mp3\r\nTitle1=Song\r\n").unwrap();

    let entries = load_playlist_file(&list).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].location, file(&track.to_string_lossy()));
    assert_eq!(entries[0].title.as_deref(), Some("Song"));
}

#[test]
fn file_urls_and_http_urls_are_handled() {
    let text = "#EXTM3U\r\n\
                file:///C:/Music/from%20url.mp3\r\n\
                file://C:/Music/plain.mp3\r\n\
                file://localhost/C:/Music/local.mp3\r\n\
                file://server/share/unc.mp3\r\n\
                http://example.com/a.mp3\r\n\
                https://example.com/b.mp3\r\n\
                ftp://example.com/c.mp3\r\n";
    let entries = read_playlist(text.as_bytes(), &base()).unwrap();
    let locations: Vec<_> = entries.iter().map(|e| e.location.clone()).collect();
    assert_eq!(
        locations,
        vec![
            file("C:/Music/from url.mp3"),
            file("C:/Music/plain.mp3"),
            file("C:/Music/local.mp3"),
            file("//server/share/unc.mp3"),
            url("http://example.com/a.mp3"),
            url("https://example.com/b.mp3"),
            url("ftp://example.com/c.mp3"),
        ]
    );
}

#[test]
fn file_urls_round_trip_through_xspf() {
    let entries = vec![LoadedEntry::new(file("C:/M\u{00FC}sik/a b#c%20d.mp3"))];
    let bytes = write_playlist(PlaylistFormat::Xspf, &entries).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains("%20"),
        "spaces must be percent-encoded: {text}"
    );
    let back = read_playlist(&text.into_bytes(), &base()).unwrap();
    assert_eq!(back, entries);
}

// ---------------------------------------------------------------------------
// encodings
// ---------------------------------------------------------------------------

#[test]
fn m3u_falls_back_to_windows_1252() {
    // 0xE4/0xF6 are 'ä'/'ö' in Windows-1252 and invalid UTF-8 on their own.
    let mut bytes = b"#EXTM3U\r\n#EXTINF:1,M".to_vec();
    bytes.push(0xE4);
    bytes.push(0x72);
    bytes.push(0xF6);
    bytes.extend_from_slice(b"schen\r\nC:\\Musik\\Lied.mp3\r\n");
    let entries = read_playlist(&bytes, &base()).unwrap();
    assert_eq!(entries[0].title.as_deref(), Some("M\u{00E4}r\u{00F6}schen"));
    assert_eq!(entries[0].location, file("C:/Musik/Lied.mp3"));
}

// ---------------------------------------------------------------------------
// malformed input
// ---------------------------------------------------------------------------

#[test]
fn malformed_input_never_panics() {
    let cases: &[(&str, &[u8])] = &[
        ("empty", b""),
        ("only a bom", b"\xEF\xBB\xBF"),
        (
            "truncated extinf",
            b"#EXTM3U\r\n#EXTINF:\r\n#EXTINF:12\r\na.mp3\r\n",
        ),
        (
            "extinf without a comma",
            b"#EXTM3U\r\n#EXTINF:5\r\na.mp3\r\n",
        ),
        (
            "negative extinf",
            b"#EXTM3U\r\n#EXTINF:-999999999999,Stream\r\nhttp://x/y\r\n",
        ),
        (
            "huge extinf",
            b"#EXTM3U\r\n#EXTINF:99999999999999999999,X\r\na.mp3\r\n",
        ),
        (
            "pls non numeric index",
            b"[playlist]\r\nFileX=a.mp3\r\nFile=a.mp3\r\nFile1=b.mp3\r\n",
        ),
        ("pls huge index", b"[playlist]\r\nFile99999999999=a.mp3\r\n"),
        ("pls no section", b"File1=a.mp3\r\nTitle1=T\r\n"),
        (
            "pls equals soup",
            b"[playlist]\r\n===\r\nFile1=\r\nLength1=\r\n",
        ),
        (
            "xspf unclosed",
            b"<playlist><trackList><track><location>a.mp3",
        ),
        (
            "xspf nested garbage",
            b"<playlist><track><track><location>a.mp3</location>",
        ),
        (
            "xspf bad entities",
            b"<playlist><track><location>&#xZZ;&amp</location></track></playlist>",
        ),
        ("xspf stray brackets", b"<<<>>><playlist/></>"),
        ("xml declaration only", b"<?xml version=\"1.0\"?>"),
        (
            "binary blob",
            &[
                0x00, 0x01, 0xFF, 0xFE, 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
            ],
        ),
        ("utf-16 like", b"\xFF\xFEa\x00.\x00m\x00p\x003\x00"),
        ("very long line", &[b'a'; 100_000]),
        ("nul in the middle", b"#EXTM3U\r\na.mp3\x00b.mp3\r\n"),
    ];

    for (name, bytes) in cases {
        // Explicit formats: the readers must be total.
        for format in [
            PlaylistFormat::M3u,
            PlaylistFormat::M3u8,
            PlaylistFormat::Pls,
            PlaylistFormat::Xspf,
        ] {
            let entries = read_playlist_as(format, bytes, &base())
                .unwrap_or_else(|e| panic!("{name} failed for {format}: {e}"));
            // Touch every field so nothing is optimized away.
            for entry in &entries {
                let _ = entry.location.to_display_string();
                let _ = entry.title.as_deref();
                let _ = entry.duration_ms;
            }
        }
        // Detection may reject, but must not panic and must not invent entries
        // from a binary blob.
        let _ = read_playlist(bytes, &base());
        let _ = read_playlist_path(bytes, &base().join("list.m3u"));
        let _ = read_playlist_path(bytes, &base().join("list.pls"));
        let _ = read_playlist_path(bytes, &base().join("list.xspf"));
        let _ = read_playlist_path(bytes, &base().join("list.b4s"));
    }
}

#[test]
fn a_binary_blob_is_rejected_without_a_hint() {
    let blob = [0x00u8, 0x01, 0x02, 0x03, 0xFF, 0xFE];
    assert!(read_playlist(&blob, &base()).is_err());
}

// ---------------------------------------------------------------------------
// loading into the core playlist
// ---------------------------------------------------------------------------

#[test]
fn loading_into_a_playlist_keeps_ids_stable_and_never_reuses_them() {
    let mut playlist = Playlist::new();
    let entries = sample_entries();
    let first = append_loaded(&mut playlist, &entries);
    assert_eq!(playlist.len(), entries.len());
    assert_eq!(
        playlist.get(first[0]).unwrap().title.as_deref(),
        Some("A Song, with a comma")
    );

    playlist.clear();
    let second = append_loaded(&mut playlist, &entries[..1]);
    assert!(second[0] > *first.last().unwrap(), "ids are never reused");
}

// ---------------------------------------------------------------------------
// performance
// ---------------------------------------------------------------------------

/// A 50 000-entry playlist must parse quickly: no quadratic behaviour and no
/// pathological allocation. The threshold is deliberately generous so a debug
/// build on a loaded CI machine still passes; the measured time is printed.
#[test]
fn fifty_thousand_entries_parse_quickly() {
    const COUNT: usize = 50_000;
    let mut text = String::with_capacity(COUNT * 64);
    text.push_str("#EXTM3U\r\n");
    for i in 0..COUNT {
        text.push_str(&format!("#EXTINF:{},Track {i}\r\n", i % 3600));
        text.push_str(&format!("C:\\Music\\Album{}\\track{i}.mp3\r\n", i % 250));
    }
    assert!(text.len() > 2_000_000);

    let started = Instant::now();
    let entries = read_playlist(text.as_bytes(), &base()).unwrap();
    let elapsed = started.elapsed();

    assert_eq!(entries.len(), COUNT);
    assert_eq!(entries[COUNT - 1].title.as_deref(), Some("Track 49999"));
    assert_eq!(entries[0].duration_ms, Some(0));
    println!("50 000-entry M3U parsed in {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(5),
        "parsing took {elapsed:?}, which is far too slow"
    );
}

/// The same size in PLS and XSPF, to show the readers stay linear there too.
#[test]
fn large_pls_and_xspf_stay_linear() {
    const COUNT: usize = 20_000;

    let mut pls = String::from("[playlist]\r\n");
    for i in 0..COUNT {
        pls.push_str(&format!("File{i}=C:\\Music\\t{i}.mp3\r\nLength{i}=1\r\n"));
    }
    let started = Instant::now();
    let entries = read_playlist_as(PlaylistFormat::Pls, pls.as_bytes(), &base()).unwrap();
    let pls_time = started.elapsed();
    assert_eq!(entries.len(), COUNT);

    let mut xspf = String::from("<playlist><trackList>");
    for i in 0..COUNT {
        xspf.push_str(&format!(
            "<track><location>C:\\Music\\t{i}.mp3</location><duration>1000</duration></track>"
        ));
    }
    xspf.push_str("</trackList></playlist>");
    let started = Instant::now();
    let entries = read_playlist_as(PlaylistFormat::Xspf, xspf.as_bytes(), &base()).unwrap();
    let xspf_time = started.elapsed();
    assert_eq!(entries.len(), COUNT);

    println!("20 000-entry PLS parsed in {pls_time:?}, XSPF in {xspf_time:?}");
    assert!(pls_time < Duration::from_secs(5), "PLS took {pls_time:?}");
    assert!(
        xspf_time < Duration::from_secs(5),
        "XSPF took {xspf_time:?}"
    );
}
