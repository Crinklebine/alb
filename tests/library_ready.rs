use lofty::{
    config::{ParseOptions, ParsingMode},
    file::TaggedFileExt,
    probe::Probe,
    tag::Accessor,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        for n in 0.. {
            let root = std::env::temp_dir().join(format!("alb-ready-{}-{n}", std::process::id()));
            if fs::create_dir(&root).is_ok() {
                fs::create_dir(root.join("in")).unwrap();
                return Self(root);
            }
        }
        unreachable!()
    }
    fn run(&self, resume: bool) -> String {
        let mut command = Command::new(env!("CARGO_BIN_EXE_alb"));
        command
            .arg("build")
            .arg("--input")
            .arg(self.0.join("in"))
            .arg("--output")
            .arg(self.0.join("out"));
        if resume {
            command.arg("--resume");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stderr).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for p in fs::read_dir(root).unwrap() {
        let p = p.unwrap().path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push(p);
        }
    }
    out
}
fn block(fields: &[(&[u8; 4], &str)]) -> Vec<u8> {
    let mut frames = vec![];
    for (id, value) in fields {
        frames.extend_from_slice(*id);
        frames.extend_from_slice(&((value.len() + 1) as u32).to_be_bytes());
        frames.extend_from_slice(&[0, 0, 0]);
        frames.extend_from_slice(value.as_bytes());
    }
    let mut bytes = b"ID3\x03\0\0".to_vec();
    bytes.extend(
        (0..4)
            .rev()
            .map(|i| ((frames.len() >> (i * 7)) & 127) as u8),
    );
    bytes.extend(frames);
    bytes
}
#[test]
fn normalizes_duplicate_tags_and_bad_dates_preserving_source_and_resume() {
    let f = Fixture::new();
    let mut bytes = block(&[
        (b"TPE1", "  Artist  "),
        (b"TIT2", "  Title  "),
        (b"TALB", "Album"),
        (b"TYER", "2013\x002013"),
        (b"TCON", "Rock"),
    ]);
    bytes.extend(block(&[(b"TPE1", " "), (b"TIT2", "Track 08")]));
    bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
    fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
    let result = f.run(false);
    assert!(result.contains("Problem files: 0"), "{result}");
    let path = files(&f.0.join("out/MP3"))
        .into_iter()
        .find(|p| p.extension().is_some_and(|e| e == "mp3"))
        .unwrap();
    let parsed = Probe::open(&path)
        .unwrap()
        .options(ParseOptions::new().parsing_mode(ParsingMode::Strict))
        .read()
        .unwrap();
    let tag = parsed.primary_tag().unwrap();
    assert_eq!(tag.artist().as_deref(), Some("Artist"));
    assert_eq!(tag.title().as_deref(), Some("Title"));
    assert_eq!(tag.album().as_deref(), Some("Album"));
    assert_eq!(tag.genre().as_deref(), Some("Rock"));
    let actual = fs::read(&path).unwrap();
    let n = actual[6..10]
        .iter()
        .fold(0usize, |n, b| (n << 7) | *b as usize);
    assert_ne!(&actual[10 + n..13 + n], b"ID3");
    assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let resumed = f.run(true);
    assert!(
        resumed.contains("1 verified existing files reused"),
        "{resumed}"
    );
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), before);
}
#[test]
fn corrects_actual_format_and_quarantines_damaged_files_without_fingerprinting() {
    let f = Fixture::new();
    fs::write(
        f.0.join("in/disguised.mp3"),
        include_bytes!("fixtures/tone.wav"),
    )
    .unwrap();
    fs::write(
        f.0.join("in/hidden.bin"),
        include_bytes!("fixtures/tone.flac"),
    )
    .unwrap();
    for (name, bytes) in [
        ("empty.mp3", vec![]),
        ("zero.flac", vec![0; 65536]),
        (
            "truncated.wav",
            include_bytes!("fixtures/tone.wav")[..100].to_vec(),
        ),
    ] {
        fs::write(f.0.join("in").join(name), bytes).unwrap();
    }
    f.run(false);
    assert!(f.0.join("out/WAV").is_dir());
    assert!(f.0.join("out/FLAC").is_dir());
    assert!(!f.0.join("out/MP3").exists());
    let problem = f.0.join("out/Problem Files/Damaged Files");
    for name in ["empty.mp3", "zero.flac", "truncated.wav"] {
        assert_eq!(
            fs::read(problem.join(name)).unwrap(),
            fs::read(f.0.join("in").join(name)).unwrap()
        );
        let explanation = fs::read_to_string(problem.join(format!("{name}.txt"))).unwrap();
        assert!(explanation.contains("media integrity error:"));
        assert!(!explanation.contains("AcoustID:"));
    }
    let report = files(&f.0.join("out/_ALB"))
        .into_iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect::<String>();
    assert!(report.contains("Format corrected:"));
}
#[test]
fn invalid_required_text_cannot_enter_main_library() {
    let f = Fixture::new();
    let mut bytes = block(&[(b"TPE1", "Artist\x01bad"), (b"TIT2", "Title")]);
    bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
    fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
    f.run(false);
    assert!(!f.0.join("out/MP3").exists());
    let copied = files(&f.0.join("out/Problem Files"))
        .into_iter()
        .find(|p| p.extension().is_some_and(|e| e == "mp3"))
        .unwrap();
    assert_eq!(fs::read(copied).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn damaged_files_never_start_fpcalc_even_with_a_key() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::write(f.0.join("in/zero.mp3"), vec![0; 65536]).unwrap();
    let fake = f.0.join("fpcalc");
    fs::write(
        &fake,
        b"#!/bin/sh\nprintf invoked > \"$0.called\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_alb"))
        .arg("build")
        .arg("--input")
        .arg(f.0.join("in"))
        .arg("--output")
        .arg(f.0.join("out"))
        .args(["--acoustid-key", "local-test-key"])
        .env("PATH", &f.0)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!f.0.join("fpcalc.called").exists());
}

#[test]
fn normalization_preserves_embedded_artwork_and_valid_unrelated_tags() {
    let f = Fixture::new();
    let picture: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let mut bytes = block(&[
        (b"TPE1", " Artist "),
        (b"TIT2", " Title "),
        (b"TCON", "Rock"),
    ]);
    let mut frame = b"\0image/png\0\x03\0".to_vec();
    frame.extend_from_slice(picture);
    bytes.extend_from_slice(b"APIC");
    bytes.extend_from_slice(&(frame.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend(frame);
    let size = bytes.len() - 10;
    for i in 0..4 {
        bytes[6 + i] = ((size >> ((3 - i) * 7)) & 127) as u8;
    }
    bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
    fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
    let result = f.run(false);
    assert!(result.contains("Problem files: 0"), "{result}");
    let path = files(&f.0.join("out/MP3"))
        .into_iter()
        .find(|p| p.extension().is_some_and(|e| e == "mp3"))
        .unwrap();
    let parsed = Probe::open(path)
        .unwrap()
        .options(ParseOptions::new().parsing_mode(ParsingMode::Strict))
        .read()
        .unwrap();
    let tag = parsed.primary_tag().unwrap();
    assert_eq!(tag.artist().as_deref(), Some("Artist"));
    assert_eq!(tag.title().as_deref(), Some("Title"));
    assert_eq!(tag.genre().as_deref(), Some("Rock"));
    assert!(tag.pictures().iter().any(|p| p.data() == picture));
    assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
}

#[test]
fn wav_external_id3_tags_are_normalized_without_changing_source() {
    for leading in [false, true] {
        let f = Fixture::new();
        let mut bytes = if leading {
            block(&[(b"TPE1", "Artist"), (b"TIT2", "Title"), (b"TALB", "Album")])
        } else {
            Vec::new()
        };
        bytes.extend_from_slice(include_bytes!("fixtures/untagged.wav"));
        let mut legacy = [0u8; 128];
        legacy[..3].copy_from_slice(b"TAG");
        legacy[3..8].copy_from_slice(b"Title");
        legacy[33..39].copy_from_slice(b"Artist");
        legacy[63..68].copy_from_slice(b"Album");
        bytes.extend_from_slice(&legacy);
        fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
        let result = f.run(false);
        assert!(result.contains("Problem files: 0"), "{result}");
        let path = files(&f.0.join("out/WAV"))
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e == "wav"))
            .unwrap();
        let output = fs::read(&path).unwrap();
        assert_eq!(&output[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(output[4..8].try_into().unwrap()) as usize + 8,
            output.len()
        );
        let parsed = Probe::open(path)
            .unwrap()
            .options(ParseOptions::new().parsing_mode(ParsingMode::Strict))
            .read()
            .unwrap();
        let tag = parsed.primary_tag().unwrap();
        assert_eq!(tag.artist().as_deref(), Some("Artist"));
        assert_eq!(tag.title().as_deref(), Some("Title"));
        assert_eq!(tag.album().as_deref(), Some("Album"));
        assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
        assert!(f.run(true).contains("1 verified existing files reused"));
    }
}

#[test]
fn wav_invalid_comment_languages_are_repaired_and_text_preserved() {
    for language in [*b"   ", [0, 1, 0], *b"eng"] {
        let f = Fixture::new();
        let mut bytes = block(&[(b"TPE1", "Artist"), (b"TIT2", "Title")]);
        let mut comment = vec![0];
        comment.extend(language);
        comment.extend_from_slice(b"Original note\0Keep this comment intact");
        bytes.extend_from_slice(b"COMM");
        bytes.extend_from_slice(&(comment.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend(comment);
        let size = bytes.len() - 10;
        for i in 0..4 {
            bytes[6 + i] = ((size >> ((3 - i) * 7)) & 127) as u8;
        }
        bytes.extend_from_slice(include_bytes!("fixtures/untagged.wav"));
        fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
        let result = f.run(false);
        assert!(result.contains("Problem files: 0"), "{result}");
        let path = files(&f.0.join("out/WAV"))
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e == "wav"))
            .unwrap();
        use lofty::file::AudioFile;
        let parsed = lofty::iff::wav::WavFile::read_from(
            &mut fs::File::open(path).unwrap(),
            ParseOptions::new().parsing_mode(ParsingMode::Strict),
        )
        .unwrap();
        let comment = parsed
            .id3v2()
            .unwrap()
            .into_iter()
            .find_map(|frame| {
                if let lofty::id3::v2::Frame::Comment(c) = frame {
                    Some(c)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(
            comment.language,
            if language == *b"eng" {
                *b"eng"
            } else {
                *b"und"
            }
        );
        assert_eq!(comment.description, "Original note");
        assert_eq!(comment.content, "Keep this comment intact");
        assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
        assert!(f.run(true).contains("1 verified existing files reused"));
    }
}

#[test]
fn failed_metadata_partial_is_removed_after_quarantine_copy() {
    for occupied in [false, true] {
        let f = Fixture::new();
        // Malformed comment forces a writer failure after the audio copy exists.
        let mut bytes = block(&[(b"TPE1", "Artist"), (b"TIT2", "Title")]);
        for language in [b"und", b"   "] {
            let mut comment = vec![0];
            comment.extend(language);
            comment.extend_from_slice(b"\0Retain both comments");
            bytes.extend_from_slice(b"COMM");
            bytes.extend_from_slice(&(comment.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&[0, 0]);
            bytes.extend(comment);
        }
        let size = bytes.len() - 10;
        for i in 0..4 {
            bytes[6 + i] = ((size >> ((3 - i) * 7)) & 127) as u8;
        }
        bytes.extend_from_slice(include_bytes!("fixtures/untagged.wav"));
        fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
        let stale =
            f.0.join("out/WAV/Artist/Loose Tracks/Title - Artist.wav.alb-partial");
        if occupied {
            fs::create_dir_all(stale.parent().unwrap()).unwrap();
            fs::write(&stale, b"previous run - do not remove").unwrap();
        }
        let result = f.run(occupied);
        assert!(result.contains("Problem files: 1"), "{result}");
        assert!(
            result.contains("0 successfully resolved; 1 unresolved (1 repair attempts failed)"),
            "{result}"
        );
        assert_eq!(
            fs::read(f.0.join("out/Problem Files/Metadata Write Errors/song.mp3")).unwrap(),
            bytes
        );
        let partials: Vec<_> = files(&f.0.join("out"))
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "alb-partial"))
            .collect();
        if occupied {
            assert_eq!(partials.as_slice(), std::slice::from_ref(&stale));
            assert_eq!(fs::read(stale).unwrap(), b"previous run - do not remove");
        } else {
            assert!(partials.is_empty(), "{partials:?}");
        }
        assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
    }
}

#[test]
fn legacy_missing_year_is_normalized_without_inventing_a_date() {
    for year in [*b"\0\0\0\0", *b"0\0\0\0", *b"    ", *b"2005"] {
        let f = Fixture::new();
        let mut bytes = block(&[(b"TPE1", "Artist"), (b"TIT2", "Title")]);
        bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
        let mut legacy = [0; 128];
        legacy[..3].copy_from_slice(b"TAG");
        legacy[3..8].copy_from_slice(b"Title");
        legacy[33..39].copy_from_slice(b"Artist");
        legacy[93..97].copy_from_slice(&year);
        bytes.extend(legacy);
        fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
        let result = f.run(false);
        assert!(result.contains("Problem files: 0"), "{result}");
        let path = files(&f.0.join("out/MP3"))
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e == "mp3"))
            .unwrap();
        let output = fs::read(&path).unwrap();
        assert_eq!(
            &output[output.len() - 35..output.len() - 31],
            if year == *b"2005" { b"2005" } else { b"0000" }
        );
        Probe::open(path)
            .unwrap()
            .options(ParseOptions::new().parsing_mode(ParsingMode::Strict))
            .read()
            .unwrap();
        assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
        assert!(f.run(true).contains("1 verified existing files reused"));
    }
}

#[test]
fn repairs_empty_frames_and_migrates_unicode_legacy_tags() {
    for case in ["unicode", "ufid", "album_artist"] {
        let f = Fixture::new();
        let title = if case == "unicode" {
            "Bishop’s Robes"
        } else {
            "Title"
        };
        let mut bytes = block(&[(b"TPE1", "Artist"), (b"TIT2", title)]);
        let title_frame = bytes.windows(4).position(|w| w == b"TIT2").unwrap();
        bytes[title_frame + 10] = 3; // UTF-8, including the curly apostrophe.
        let (id, data): (&[u8; 4], &[u8]) = match case {
            "ufid" => (b"UFID", &[0]),
            "comment" => (b"COMM", &[0, 0]),
            "album_artist" => (b"TPE2", &[0]),
            _ => (b"TCON", b"\0Rock"),
        };
        bytes.extend_from_slice(id);
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(&[0; 64]); // Typical tag padding permits best-attempt metadata recovery.
        let size = bytes.len() - 10;
        for i in 0..4 {
            bytes[6 + i] = ((size >> ((3 - i) * 7)) & 127) as u8;
        }
        bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
        {
            let mut legacy = [0; 128];
            legacy[..3].copy_from_slice(b"TAG");
            legacy[3..8].copy_from_slice(b"Title");
            legacy[33..39].copy_from_slice(b"Artist");
            legacy[93..97].copy_from_slice(b"2005");
            legacy[97..110].copy_from_slice(b"Unique legacy");
            bytes.extend(legacy);
        }
        fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
        let result = f.run(false);
        assert!(result.contains("Problem files: 0"), "{case}: {result}");
        let path = files(&f.0.join("out/MP3"))
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e == "mp3"))
            .unwrap();
        let parsed = Probe::open(&path)
            .unwrap()
            .options(ParseOptions::new().parsing_mode(ParsingMode::Strict))
            .read()
            .unwrap();
        assert_eq!(
            parsed.primary_tag().unwrap().title().as_deref(),
            Some(title)
        );
        if case == "unicode" {
            assert!(parsed.tag(lofty::tag::TagType::Id3v1).is_none());
            assert_eq!(
                parsed.primary_tag().unwrap().comment().as_deref(),
                Some("Unique legacy")
            );
        }
        assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
        assert!(f.run(true).contains("1 verified existing files reused"));
    }
}

#[test]
fn legacy_mp3ext_padding_is_normalized_and_resume_works() {
    let f = Fixture::new();
    let mut bytes = block(&[
        (b"TPE1", "Artist"),
        (b"TIT2", "Title"),
        (b"TALB", "Album"),
        (b"TPE2", ""),
    ]);
    bytes.extend((0..1073).map(|i| b"MP3ext "[i % 7]));
    let size = bytes.len() - 10;
    for i in 0..4 {
        bytes[6 + i] = ((size >> ((3 - i) * 7)) & 127) as u8;
    }
    bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
    fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
    let result = f.run(false);
    assert!(result.contains("Problem files: 0"), "{result}");
    let path = files(&f.0.join("out/MP3"))
        .into_iter()
        .find(|p| p.extension().is_some_and(|e| e == "mp3"))
        .unwrap();
    let parsed = Probe::open(path)
        .unwrap()
        .options(ParseOptions::new().parsing_mode(ParsingMode::Strict))
        .read()
        .unwrap();
    let tag = parsed.primary_tag().unwrap();
    assert_eq!(tag.title().as_deref(), Some("Title"));
    assert_eq!(tag.artist().as_deref(), Some("Artist"));
    assert_eq!(tag.album().as_deref(), Some("Album"));
    assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
    assert!(f.run(true).contains("1 verified existing files reused"));
}

#[test]
fn actual_video_tracks_are_preserved_as_unknown_even_with_audio_extension() {
    fn atom(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut b = ((data.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend(kind);
        b.extend(data);
        b
    }
    for video in [true, false] {
        let f = Fixture::new();
        let mut bytes = include_bytes!("fixtures/tone.m4a").to_vec();
        let mut handler = vec![0; 8];
        handler.extend_from_slice(b"vide");
        let data = if video {
            atom(b"trak", &atom(b"mdia", &atom(b"hdlr", &handler)))
        } else {
            atom(b"udta", &atom(b"covr", &handler))
        };
        bytes.extend(atom(b"moov", &data));
        fs::write(f.0.join("in/movie.m4a"), &bytes).unwrap();
        let result = f.run(false);
        assert!(result.contains("Problem files: 0"), "{result}");
        if video {
            assert_eq!(fs::read(f.0.join("out/UNKNOWN/movie.m4a")).unwrap(), bytes);
        } else {
            assert!(f.0.join("out/M4A").exists());
        }
    }
}

#[test]
fn long_generated_names_shorten_without_losing_tags_and_collisions_stay_distinct() {
    let f = Fixture::new();
    let artist = "Artist".repeat(12);
    let title = "Title".repeat(22);
    for suffix in ["a", "b"] {
        let mut bytes = block(&[
            (b"TPE1", &artist),
            (b"TIT2", &format!("{title}{suffix}")),
            (b"TALB", "An Album"),
        ]);
        bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
        fs::write(f.0.join(format!("in/{suffix}.mp3")), bytes).unwrap();
    }
    let result = f.run(false);
    assert!(result.contains("Problem files: 0"), "{result}");
    let songs: Vec<_> = files(&f.0.join("out/MP3"))
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "mp3"))
        .collect();
    assert_eq!(songs.len(), 2);
    for path in songs {
        assert!(path.as_os_str().as_encoded_bytes().len() <= 240);
        let tag = Probe::open(path).unwrap().read().unwrap();
        assert_eq!(
            tag.primary_tag().unwrap().artist().as_deref(),
            Some(artist.as_str())
        );
        assert!(
            tag.primary_tag()
                .unwrap()
                .title()
                .unwrap()
                .starts_with(&title)
        );
    }
    assert!(f.run(true).contains("2 verified existing files reused"));
}

#[test]
fn prior_problem_wrappers_are_removed_only_with_matching_alb_report() {
    for report_style in [0, 1, 2] {
        let recognized = report_style != 0;
        let f = Fixture::new();
        let rel = "Problem Files/Missing Metadata/Problem Files/Metadata Errors/Band/song.mp3";
        let source = f.0.join("in").join(rel);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, include_bytes!("fixtures/untagged.mp3")).unwrap();
        if recognized {
            fs::write(
                source.with_extension("mp3.txt"),
                if report_style == 1 {
                    format!("ALB problem file\nDestination: Some({source:?})\n")
                } else {
                    format!("ALB problem file\n\nFILE LOCATIONS\n==============\nDestination: {source:?}\n")
                },
            )
            .unwrap();
        }
        f.run(false);
        let dest =
            f.0.join("out/Problem Files/Missing Metadata")
                .join(if recognized { "Band/song.mp3" } else { rel });
        assert!(dest.exists(), "{dest:?}");
    }
}

#[test]
fn missing_artist_takes_priority_over_repairable_legacy_year_error() {
    let f = Fixture::new();
    let mut bytes = block(&[(b"TIT2", "Nature")]);
    bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
    let mut legacy = [0; 128];
    legacy[..3].copy_from_slice(b"TAG");
    legacy[3..9].copy_from_slice(b"Nature");
    bytes.extend(legacy);
    fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
    let result = f.run(false);
    assert!(result.contains("Problem files: 1"), "{result}");
    let report =
        fs::read_to_string(f.0.join("out/Problem Files/Missing Metadata/song.mp3.txt")).unwrap();
    assert!(report.contains("missing artist"));
    assert!(report.contains("year"));
}

#[test]
fn zero_disc_number_is_removed_without_losing_track_identity() {
    let f = Fixture::new();
    let mut bytes = block(&[
        (b"TPE1", "The Germs"),
        (b"TIT2", "Lexicon Devil"),
        (b"TALB", "Anthology"),
        (b"TRCK", "03"),
        (b"TPOS", "0"),
    ]);
    bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
    fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
    let result = f.run(false);
    assert!(result.contains("Problem files: 0"), "{result}");
    let path =
        f.0.join("out/MP3/The Germs/Anthology/03 - Lexicon Devil - The Germs.mp3");
    let parsed = Probe::open(path).unwrap().read().unwrap();
    assert_eq!(parsed.primary_tag().unwrap().disk(), None);
    assert_eq!(parsed.primary_tag().unwrap().track(), Some(3));
    assert_eq!(fs::read(f.0.join("in/song.mp3")).unwrap(), bytes);
    assert!(f.run(true).contains("1 verified existing files reused"));
}

#[test]
fn non_audio_riff_and_utf16_files_are_preserved_without_problem_reports() {
    let f = Fixture::new();
    let text: Vec<u8> = "The Clash - London Calling"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut utf16 = vec![255, 254];
    utf16.extend(text);
    for (name, bytes) in [
        ("track.txt", utf16),
        ("cover.webp", b"RIFF\x04\0\0\0WEBP".to_vec()),
        ("movie.avi", b"RIFF\x04\0\0\0AVI ".to_vec()),
    ] {
        fs::write(f.0.join("in").join(name), bytes).unwrap();
    }
    let result = f.run(false);
    assert!(result.contains("Problem files: 0"), "{result}");
    for name in ["track.txt", "cover.webp", "movie.avi"] {
        assert_eq!(
            fs::read(f.0.join("in").join(name)).unwrap(),
            fs::read(f.0.join("out/UNKNOWN").join(name)).unwrap()
        );
    }
}

#[test]
fn missing_metadata_reports_name_every_missing_required_field() {
    for (artist, title) in [(false, false), (true, false), (false, true)] {
        let f = Fixture::new();
        let mut fields = Vec::new();
        if artist {
            fields.push((b"TPE1", "Artist"));
        }
        if title {
            fields.push((b"TIT2", "Title"));
        }
        let mut bytes = block(&fields);
        bytes.extend_from_slice(include_bytes!("fixtures/untagged.mp3"));
        fs::write(f.0.join("in/song.mp3"), &bytes).unwrap();
        f.run(false);
        let report =
            fs::read_to_string(f.0.join("out/Problem Files/Missing Metadata/song.mp3.txt"))
                .unwrap();
        assert!(report.contains("Reason: Required metadata"), "{report}");
        assert_eq!(
            report.contains("Missing required metadata: artist"),
            !artist,
            "{report}"
        );
        assert_eq!(
            report.contains("Missing required metadata: title"),
            !title,
            "{report}"
        );
    }
}

#[test]
fn reprocessing_archives_alb_reports_and_keeps_unknown_paths_stable() {
    let f = Fixture::new();
    let input = f.0.join("in");
    fs::write(
        input.join("song.mp3"),
        include_bytes!("fixtures/untagged.mp3"),
    )
    .unwrap();
    fs::write(input.join("notes.txt"), b"personal notes").unwrap();
    fs::create_dir(input.join("_ALB")).unwrap();
    fs::write(input.join("_ALB/build-personal.txt"), b"not an ALB report").unwrap();
    fs::create_dir_all(input.join("Problem Files/Missing Metadata")).unwrap();
    fs::write(
        input.join("Problem Files/Missing Metadata/personal.txt"),
        b"personal explanation",
    )
    .unwrap();
    f.run(false);
    let first = f.0.join("out");
    let original_reports: Vec<_> = files(&first)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .filter(|p| {
            let text = fs::read_to_string(p).unwrap();
            text.starts_with("ALB BUILD REPORT v1\n") || text.starts_with("ALB problem file\n")
        })
        .map(|p| fs::read(p).unwrap())
        .collect();
    assert_eq!(original_reports.len(), 2);

    // Include pollution produced by older versions, including a detached sidecar.
    fs::create_dir_all(first.join("UNKNOWN/UNKNOWN")).unwrap();
    fs::write(first.join("UNKNOWN/UNKNOWN/old.txt"), b"old user notes").unwrap();
    fs::create_dir_all(first.join("UNKNOWN/Problem Files/Missing Metadata")).unwrap();
    fs::copy(
        first.join("Problem Files/Missing Metadata/song.mp3.txt"),
        first.join("UNKNOWN/Problem Files/Missing Metadata/song.mp3.txt"),
    )
    .unwrap();

    let mut source = first;
    for pass in ["second", "third"] {
        let output = f.0.join(pass);
        let before: Vec<_> = files(&source)
            .into_iter()
            .map(|p| (p.clone(), fs::read(&p).unwrap()))
            .collect();
        let result = Command::new(env!("CARGO_BIN_EXE_alb"))
            .args(["build", "--input"])
            .arg(&source)
            .arg("--output")
            .arg(&output)
            .arg("--no-fingerprint-cache")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        for (path, bytes) in before {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        for (relative, bytes) in [
            ("notes.txt", b"personal notes".as_slice()),
            ("old.txt", b"old user notes"),
            ("_ALB/build-personal.txt", b"not an ALB report"),
            (
                "Problem Files/Missing Metadata/personal.txt",
                b"personal explanation",
            ),
        ] {
            assert_eq!(
                fs::read(output.join("UNKNOWN").join(relative)).unwrap(),
                bytes
            );
        }
        assert!(!output.join("UNKNOWN/UNKNOWN").exists());
        assert!(
            !output
                .join("UNKNOWN/Problem Files/Missing Metadata/song.mp3.txt")
                .exists()
        );
        let archive: Vec<_> = files(&output.join("_ALB/Previous Reports"))
            .into_iter()
            .map(|p| fs::read(p).unwrap())
            .collect();
        for report in &original_reports {
            assert!(archive.contains(report));
        }
        assert!(
            output
                .join("Problem Files/Missing Metadata/song.mp3.txt")
                .exists()
        );
        let resumed = Command::new(env!("CARGO_BIN_EXE_alb"))
            .args(["build", "--input"])
            .arg(&source)
            .arg("--output")
            .arg(&output)
            .args(["--resume", "--no-fingerprint-cache"])
            .output()
            .unwrap();
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert!(String::from_utf8_lossy(&resumed.stderr).contains("0 verified copies"));
        source = output;
    }
}

#[test]
fn unverified_alb_folder_names_do_not_change_unknown_routing() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("in/UNKNOWN")).unwrap();
    fs::create_dir_all(f.0.join("in/_ALB")).unwrap();
    fs::write(f.0.join("in/UNKNOWN/notes.txt"), b"user folder").unwrap();
    fs::write(f.0.join("in/_ALB/build-user.txt"), b"ordinary text").unwrap();
    f.run(false);
    assert_eq!(
        fs::read(f.0.join("out/UNKNOWN/UNKNOWN/notes.txt")).unwrap(),
        b"user folder"
    );
    assert_eq!(
        fs::read(f.0.join("out/UNKNOWN/_ALB/build-user.txt")).unwrap(),
        b"ordinary text"
    );
    assert!(!f.0.join("out/_ALB/Previous Reports").exists());
}
