//! Metadata gate and normalization on newly-created output handles only.
use crate::candidates::FileType as Kind;
use lofty::{
    config::{ParseOptions, ParsingMode, WriteOptions},
    file::{AudioFile, FileType, TaggedFileExt},
    probe::Probe,
    tag::{Accessor, ItemKey, Tag, TagExt, TagType},
};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetadataUpdate {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    /// True for the library gate; false for legacy isolated field-update tests.
    pub normalize: bool,
}
fn clean(value: Option<&str>) -> Option<String> {
    use unicode_normalization::UnicodeNormalization;
    value
        .map(|v| {
            v.trim_matches(|c: char| c.is_whitespace() || c == '\0')
                .nfc()
                .collect::<String>()
        })
        .filter(|v| !v.is_empty())
}
pub fn prepare(track: &mut crate::catalog::Track) {
    if track.file_type == Kind::Unknown {
        return;
    }
    if track.disc_number == Some(0) {
        track.disc_number = None;
        track
            .metadata_notes
            .push("Metadata warning: zero disc number treated as unspecified".into());
    }
    track.artist = clean(track.artist.as_deref());
    track.title = clean(track.title.as_deref());
    track.album = clean(track.album.as_deref());
    track.album_artist = clean(track.album_artist.as_deref());
    if [
        &track.artist,
        &track.title,
        &track.album,
        &track.album_artist,
    ]
    .into_iter()
    .flatten()
    .any(|v| v.chars().any(char::is_control))
    {
        track.metadata_notes.push(
            "metadata/read error: embedded control characters in sorting metadata require review"
                .into(),
        );
        track.metadata_update = None;
        return;
    }
    if track.artist.is_some() && track.title.is_some() {
        track.metadata_update = Some(MetadataUpdate {
            artist: track.artist.clone(),
            title: track.title.clone(),
            album: track.album.clone(),
            album_artist: track.album_artist.clone(),
            normalize: true,
        });
        track.metadata_notes.push("Metadata gate: require Artist/Title, preserve available Album, normalize output if needed, and verify strict read-back plus encoded audio integrity".into());
    }
}
fn options(mode: ParsingMode) -> ParseOptions {
    ParseOptions::new()
        .parsing_mode(mode)
        .read_cover_art(true)
        .max_junk_bytes(16 * 1024 * 1024)
}
fn write_options() -> WriteOptions {
    WriteOptions::new().parse_options(options(ParsingMode::Relaxed))
}
fn detail(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        text.push_str(": ");
        text.push_str(&e.to_string());
        source = e.source();
    }
    text
}
fn check(
    file: &mut File,
    expected: &MetadataUpdate,
    kind: Kind,
) -> Result<(), Box<dyn std::error::Error>> {
    if kind == Kind::Wav {
        let mut header = [0; 8];
        file.rewind()?;
        file.read_exact(&mut header)?;
        if u32::from_le_bytes(header[4..8].try_into().unwrap()) as u64 + 8 != file.metadata()?.len()
        {
            return Err("WAV RIFF size requires normalization".into());
        }
    }
    file.rewind()?;
    let actual = probe(file, kind)?
        .options(options(ParsingMode::Strict))
        .read()?;
    if kind == Kind::Mp3 && crate::inspection::mp3_blocks(file)?.len() > 1 {
        return Err("duplicate ID3v2 blocks require consolidation".into());
    }
    let tag = if kind == Kind::Wav {
        actual
            .tag(TagType::RiffInfo)
            .or_else(|| actual.primary_tag())
    } else {
        actual.primary_tag()
    }
    .ok_or("primary metadata tag missing")?;
    for (field, wanted, got) in [
        ("Artist", expected.artist.as_deref(), tag.artist()),
        ("Title", expected.title.as_deref(), tag.title()),
        ("Album", expected.album.as_deref(), tag.album()),
    ] {
        if let Some(wanted) = wanted
            && got.as_deref() != Some(wanted)
        {
            return Err(format!("{field} did not round-trip").into());
        }
    }
    for other in actual.tags() {
        if other.disk() == Some(0) {
            return Err("zero disc number requires normalization".into());
        }
        for (field, wanted, got) in [
            ("Artist", expected.artist.as_deref(), other.artist()),
            ("Title", expected.title.as_deref(), other.title()),
            ("Album", expected.album.as_deref(), other.album()),
        ] {
            if let (Some(wanted), Some(got)) = (wanted, got) {
                let legacy_prefix = other.tag_type() == TagType::Id3v1
                    && got.len() >= 28
                    && wanted.starts_with(got.as_ref());
                if !got.trim().is_empty() && got != wanted && !legacy_prefix {
                    return Err(format!("conflicting {field} in {:?}", other.tag_type()).into());
                }
            }
        }
    }
    if expected.normalize {
        if tag.artist().is_none_or(|s| s.trim().is_empty())
            || tag.title().is_none_or(|s| s.trim().is_empty())
        {
            return Err("missing required Artist or Title".into());
        }
        let album_artist = tag.get_string(ItemKey::AlbumArtist).or_else(|| {
            actual
                .tags()
                .iter()
                .find_map(|tag| tag.get_string(ItemKey::AlbumArtist))
        });
        if expected.album_artist.as_deref() != album_artist {
            return Err("Album Artist did not round-trip".into());
        }
        if [&tag.artist(), &tag.title(), &tag.album()]
            .iter()
            .filter_map(|v| v.as_ref())
            .any(|v| v.chars().any(char::is_control))
        {
            return Err("invalid control characters in sorting metadata".into());
        }
    }
    Ok(())
}
fn artwork(
    file: &mut File,
    kind: Kind,
) -> Result<std::collections::BTreeSet<[u8; 32]>, Box<dyn std::error::Error>> {
    file.rewind()?;
    let parsed = if kind == Kind::Wav {
        let layout = crate::media::wav_layout(file)?;
        Probe::with_file_type(
            crate::media::Window::new(file, layout.start, layout.end)?,
            FileType::Wav,
        )
        .options(options(ParsingMode::Relaxed))
        .read()?
    } else {
        probe(file, kind)?
            .options(options(ParsingMode::Relaxed))
            .read()?
    };
    let extra = if matches!(kind, Kind::Mp3 | Kind::Wav) {
        crate::inspection::mp3_blocks_with_cover(file, true)?
    } else {
        Vec::new()
    };
    Ok(parsed
        .tags()
        .iter()
        .chain(extra.iter())
        .flat_map(|tag| tag.pictures())
        .map(|pic| *blake3::hash(pic.data()).as_bytes())
        .collect())
}

pub fn apply(file: &mut File, update: &MetadataUpdate, kind: Kind) -> io::Result<bool> {
    let mut stage = "audio payload check";
    let result = (|| -> Result<bool, Box<dyn std::error::Error>> {
        crate::media::structure(file, kind)?;
        if check(file, update, kind).is_ok() {
            return Ok(false);
        }
        let before = crate::media::payload(file, kind)?;
        stage = "reading existing tags for normalization";
        if kind == Kind::Mp3 {
            remove_empty_broken_frames(file)?;
        }
        let original_artwork = artwork(file, kind)?;
        file.rewind()?;
        if kind == Kind::Mp3 {
            normalize_mp3(file, update)?;
        } else {
            if kind == Kind::Wav {
                normalize_wav_layout(file, update)?;
            }
            file.rewind()?;
            let mut tagged = probe(file, kind)?
                .options(options(ParsingMode::Relaxed))
                .read()?;
            let tag_kind = if kind == Kind::Wav {
                TagType::RiffInfo
            } else {
                tagged.primary_tag_type()
            };
            if tagged.tag(tag_kind).is_none() {
                tagged.insert_tag(Tag::new(tag_kind));
            }
            let tag = tagged.tag_mut(tag_kind).ok_or("no writable tag")?;
            if let Some(s) = &update.artist {
                tag.set_artist(s.clone());
            }
            if let Some(s) = &update.title {
                tag.set_title(s.clone());
            }
            if let Some(s) = &update.album {
                tag.set_album(s.clone());
            }
            if let Some(s) = &update.album_artist {
                tag.insert_text(ItemKey::AlbumArtist, s.clone());
            }
            if tag.disk() == Some(0) {
                tag.remove_disk();
            }
            stage = "writing normalized tags";
            file.rewind()?;
            tag.save_to(&mut *file, write_options())?;
        }
        if kind == Kind::Wav {
            let size = u32::try_from(
                file.metadata()?
                    .len()
                    .checked_sub(8)
                    .ok_or("invalid WAV size")?,
            )?;
            file.seek(SeekFrom::Start(4))?;
            file.write_all(&size.to_le_bytes())?;
        }
        stage = "strict metadata read-back";
        file.sync_all()?;
        check(file, update, kind)?;
        stage = "artwork preservation";
        if !original_artwork.is_subset(&artwork(file, kind)?) {
            return Err("artwork was lost during normalization".into());
        }
        stage = "encoded audio preservation";
        if before != crate::media::payload(file, kind)? {
            return Err("encoded audio payload changed during metadata normalization".into());
        }
        Ok(true)
    })();
    result.map_err(|e| {
        io::Error::other(format!(
            "metadata update failed: {stage}: {}",
            detail(e.as_ref())
        ))
    })
}
/// Replace only the leading tag region, using bounded memory and the same open handle.
fn replace_prefix(file: &mut File, old: u64, new: &[u8]) -> io::Result<()> {
    let len = file.metadata()?.len();
    let new_len = new.len() as u64;
    let mut buf = [0; 65536];
    if new_len > old {
        let delta = new_len - old;
        file.set_len(
            len.checked_add(delta)
                .ok_or_else(|| io::Error::other("tag size overflow"))?,
        )?;
        let mut end = len;
        while end > old {
            let n = (end - old).min(buf.len() as u64) as usize;
            let start = end - n as u64;
            file.seek(SeekFrom::Start(start))?;
            file.read_exact(&mut buf[..n])?;
            file.seek(SeekFrom::Start(start + delta))?;
            file.write_all(&buf[..n])?;
            end = start;
        }
    } else {
        let mut start = old;
        while start < len {
            let n = (len - start).min(buf.len() as u64) as usize;
            file.seek(SeekFrom::Start(start))?;
            file.read_exact(&mut buf[..n])?;
            file.seek(SeekFrom::Start(start - old + new_len))?;
            file.write_all(&buf[..n])?;
            start += n as u64;
        }
        file.set_len(len - old + new_len)?;
    }
    file.rewind()?;
    file.write_all(new)?;
    file.rewind()
}
// Move external ID3 into a standard WAV ID3 chunk, without touching fmt/data bytes.
// An invalid language cannot identify the comment's actual language. Use ISO 639
// "und" (undetermined), retaining the description and text byte-for-byte as parsed.
fn normalize_comment_languages(
    tag: &mut lofty::id3::v2::Id3v2Tag,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut identities = std::collections::HashSet::new();
    let mut collision = false;
    tag.retain_mut(|frame| {
        if let lofty::id3::v2::Frame::Comment(comment) = frame {
            if !comment.language.iter().all(u8::is_ascii_alphabetic) {
                comment.language = *b"und";
            }
            if !identities.insert((comment.language, comment.description.to_string())) {
                collision = true;
            }
        }
        true
    });
    if collision {
        return Err("comment language repair would create duplicate comment identities; manual review required".into());
    }
    Ok(())
}

fn normalize_wav_layout(
    file: &mut File,
    update: &MetadataUpdate,
) -> Result<(), Box<dyn std::error::Error>> {
    use lofty::id3::v2::Id3v2Tag;
    let layout = crate::media::wav_layout(file)?;
    if layout.start == 0 && !layout.legacy_tag {
        return Ok(());
    }
    let mut tag = Id3v2Tag::new();
    if layout.start > 0 {
        let mut prefix = vec![0; usize::try_from(layout.start)?];
        file.rewind()?;
        file.read_exact(&mut prefix)?;
        let parsed = lofty::mpeg::MpegFile::read_from(
            &mut io::Cursor::new(prefix),
            options(ParsingMode::Relaxed).read_properties(false),
        )?;
        if let Some(existing) = parsed.id3v2() {
            tag = existing.clone();
        }
    }
    if layout.legacy_tag {
        // Leave room for the MPEG reader to probe optional footer formats.
        let mut bytes = [0; 384];
        file.seek(SeekFrom::Start(layout.end))?;
        file.read_exact(&mut bytes[256..])?;
        let parsed = lofty::mpeg::MpegFile::read_from(
            &mut io::Cursor::new(bytes),
            options(ParsingMode::Relaxed).read_properties(false),
        )?;
        if let Some(legacy) = parsed.id3v1() {
            let generic: Tag = legacy.clone().into();
            let converted: Id3v2Tag = generic.into();
            // Sort converted frames for reproducible resume comparisons.
            let mut frames: Vec<_> = converted.into_iter().collect();
            frames.sort_by(|a, b| a.id().as_str().cmp(b.id().as_str()));
            for frame in frames {
                if let Some(prior) = tag.insert(frame) {
                    tag.insert(prior);
                }
            }
        }
    }
    if let Some(s) = &update.artist {
        tag.set_artist(s.clone());
    }
    if let Some(s) = &update.title {
        tag.set_title(s.clone());
    }
    if let Some(s) = &update.album {
        tag.set_album(s.clone());
    }
    if tag.disk() == Some(0) {
        tag.remove_disk();
    }
    normalize_comment_languages(&mut tag)?;
    let mut chunk = Vec::new();
    tag.dump_to(&mut chunk, write_options())?;
    // Remove external bytes from this private partial only; audio is checked before publication.
    file.set_len(layout.end)?;
    replace_prefix(file, layout.start, &[])?;
    file.seek(SeekFrom::End(0))?;
    file.write_all(b"id3 ")?;
    file.write_all(&u32::try_from(chunk.len())?.to_le_bytes())?;
    file.write_all(&chunk)?;
    if chunk.len() % 2 != 0 {
        file.write_all(&[0])?;
    }
    let size = u32::try_from(file.metadata()?.len() - 8)?;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&size.to_le_bytes())?;
    file.rewind()?;
    Ok(())
}

// Only remove provably empty malformed frames, before the tag parser encounters them.
// Keep every other byte (including artwork) unchanged. Unsupported layouts are untouched.
fn remove_empty_broken_frames(file: &mut File) -> io::Result<()> {
    let end = crate::media::id3_end(file)?;
    file.rewind()?;
    let mut bytes = vec![0; usize::try_from(end).map_err(io::Error::other)?];
    file.read_exact(&mut bytes)?;
    let mut offset = 0;
    let mut repaired = Vec::new();
    while offset < bytes.len() {
        let header = &bytes[offset..offset + 10];
        let size = header[6..10]
            .iter()
            .fold(0usize, |a, b| (a << 7) | *b as usize);
        let block_end = offset + 10 + size;
        let footer = if header[3] == 4 && header[5] & 16 != 0 {
            10
        } else {
            0
        };
        if !matches!(header[3], 3 | 4) || header[5] != 0 {
            repaired.extend_from_slice(&bytes[offset..block_end + footer]);
        } else {
            let mut body = Vec::new();
            let mut pos = offset + 10;
            while pos < block_end && bytes[pos] != 0 {
                let remaining = &bytes[pos..block_end];
                // MP3ext used repeated text (sometimes ending mid-repeat) as padding.
                // Recognize only an exact trailing pattern at a frame boundary.
                if remaining.starts_with(b"MP3ext ")
                    && remaining
                        .iter()
                        .enumerate()
                        .all(|(i, b)| *b == b"MP3ext "[i % 7])
                {
                    body.resize(body.len() + remaining.len(), 0);
                    pos = block_end;
                    break;
                }
                if pos + 10 > block_end {
                    break;
                }
                let frame = &bytes[pos..pos + 10];
                let length = if header[3] == 4 {
                    if frame[4..8].iter().any(|b| b & 128 != 0) {
                        return Err(io::Error::other("invalid ID3 frame size"));
                    }
                    frame[4..8]
                        .iter()
                        .fold(0usize, |a, b| (a << 7) | *b as usize)
                } else {
                    u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize
                };
                let next = pos
                    .checked_add(10)
                    .and_then(|n| n.checked_add(length))
                    .filter(|n| *n <= block_end)
                    .ok_or_else(|| io::Error::other("ID3 frame exceeds tag boundary"))?;
                let data = &bytes[pos + 10..next];
                let empty_invalid = frame[8..10] == [0, 0]
                    && data.iter().all(|b| *b == 0)
                    && ((&frame[..4] == b"UFID" && data.len() <= 1)
                        || (&frame[..4] == b"COMM" && data.len() < 4));
                if !empty_invalid {
                    body.extend_from_slice(&bytes[pos..next]);
                }
                pos = next;
            }
            body.extend_from_slice(&bytes[pos..block_end]);
            let mut h = header.to_vec();
            for i in 0..4 {
                h[6 + i] = ((body.len() >> (7 * (3 - i))) & 127) as u8;
            }
            repaired.extend(h);
            repaired.extend(body);
        }
        offset = block_end + footer;
    }
    if repaired != bytes {
        replace_prefix(file, end, &repaired)?;
    }
    Ok(())
}

fn normalize_mp3(
    file: &mut File,
    update: &MetadataUpdate,
) -> Result<(), Box<dyn std::error::Error>> {
    use lofty::id3::v2::Id3v2Tag;
    let end = crate::media::id3_end(file)?;
    file.rewind()?;
    let mut mpeg = lofty::mpeg::MpegFile::read_from(&mut *file, options(ParsingMode::Relaxed))?;
    let mut tag = Id3v2Tag::new();
    let mut offset = 0;
    while offset < end {
        let mut h = [0; 10];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut h)?;
        let size = h[6..].iter().fold(0usize, |a, b| (a << 7) | *b as usize);
        let mut bytes = h.to_vec();
        bytes.resize(10 + size, 0);
        file.read_exact(&mut bytes[10..])?;
        let block = lofty::mpeg::MpegFile::read_from(
            &mut io::Cursor::new(bytes),
            options(ParsingMode::Relaxed).read_properties(false),
        )?;
        if let Some(native) = block.id3v2() {
            for frame in native.clone() {
                let copy = frame.clone();
                if let Some(previous) = tag.insert(frame) {
                    if copy.id().as_str() == "APIC" && copy != previous {
                        return Err("conflicting artwork requires manual review".into());
                    }
                    tag.insert(previous);
                }
            }
        }
        offset += 10 + size as u64 + if h[3] == 4 && h[5] & 16 != 0 { 10 } else { 0 };
    }
    if end == 0
        && let Some(native) = mpeg.id3v2()
    {
        tag = native.clone();
    }
    if let Some(s) = &update.artist {
        tag.set_artist(s.clone());
    }
    if let Some(s) = &update.title {
        tag.set_title(s.clone());
    }
    if let Some(s) = &update.album {
        tag.set_album(s.clone());
    }
    if let Some(s) = &update.album_artist {
        tag.insert(lofty::id3::v2::Frame::Text(
            lofty::id3::v2::TextInformationFrame::new(
                lofty::id3::v2::FrameId::Valid("TPE2".into()),
                lofty::TextEncoding::UTF8,
                s.clone(),
            ),
        ));
    }
    if update.album_artist.is_none() {
        tag.retain(|frame| !matches!(frame, lofty::id3::v2::Frame::Text(t) if t.id().as_str() == "TPE2" && t.value.trim_matches(|c: char| c.is_whitespace() || c == '\0').is_empty()));
    }
    let remove_legacy = [&update.artist, &update.title, &update.album]
        .into_iter()
        .flatten()
        .any(|s| s.chars().any(|c| c as u32 > 255));
    if remove_legacy && let Some(legacy) = mpeg.id3v1() {
        let generic: Tag = legacy.clone().into();
        let converted: Id3v2Tag = generic.into();
        let mut frames: Vec<_> = converted.into_iter().collect();
        frames.sort_by(|a, b| a.id().as_str().cmp(b.id().as_str()));
        for frame in frames {
            let mut copy = frame.clone();
            if let Some(prior) = tag.insert(frame) {
                tag.insert(prior);
                if let lofty::id3::v2::Frame::Comment(comment) = &mut copy {
                    // Preserve a legacy comment even when ID3v2 already has one.
                    for n in 0..1000 {
                        comment.description = format!("Legacy ID3v1 comment {n}").into();
                        if !(&tag).into_iter().any(|f| matches!(f, lofty::id3::v2::Frame::Comment(c) if c.language == comment.language && c.description == comment.description)) {
                            break;
                        }
                    }
                    tag.insert(copy);
                }
            }
        }
    }
    if tag.disk() == Some(0) {
        tag.remove_disk();
    }
    normalize_comment_languages(&mut tag)?;
    let mut bytes = Vec::new();
    tag.dump_to(&mut bytes, write_options())?;
    replace_prefix(file, end, &bytes)?;
    // Keep legacy fields, synchronizing identity rather than leaving stale alternatives.
    if remove_legacy && mpeg.id3v1().is_some() {
        let len = file.metadata()?.len();
        file.seek(SeekFrom::End(-128))?;
        let mut marker = [0; 3];
        file.read_exact(&mut marker)?;
        if &marker != b"TAG" {
            return Err("legacy tag location changed".into());
        }
        file.set_len(len - 128)?;
    } else if let Some(legacy) = mpeg.id3v1_mut() {
        if let Some(s) = &update.artist {
            legacy.set_artist(s.clone());
        }
        if let Some(s) = &update.title {
            legacy.set_title(s.clone());
        }
        if let Some(s) = &update.album {
            legacy.set_album(s.clone());
        }
        file.rewind()?;
        // ID3v1 uses four ASCII zeroes for an unknown year. Lofty's writer
        // emits NUL bytes for None, which its strict reader rejects.
        if legacy.year.is_none() {
            legacy.year = Some(0);
        }
        legacy.save_to(&mut *file, write_options())?;
    }
    if let Some(ape) = mpeg.ape_mut() {
        if let Some(s) = &update.artist {
            ape.set_artist(s.clone());
        }
        if let Some(s) = &update.title {
            ape.set_title(s.clone());
        }
        if let Some(s) = &update.album {
            ape.set_album(s.clone());
        }
        file.rewind()?;
        ape.save_to(&mut *file, write_options())?;
    }
    Ok(())
}
fn probe(file: &mut File, kind: Kind) -> io::Result<Probe<&mut File>> {
    let ty = match kind {
        Kind::Mp3 => FileType::Mpeg,
        Kind::M4a => FileType::Mp4,
        Kind::Flac => FileType::Flac,
        Kind::Wav => FileType::Wav,
        _ => return Probe::new(file).guess_file_type(),
    };
    Ok(Probe::with_file_type(file, ty))
}

#[cfg(test)]
mod tests {
    #[test]
    fn padding_recognition_does_not_hide_bad_frame_boundaries() {
        for body in [
            b"TIT2\0\0\x10\0\0\0MP3ext MP3ext ".as_slice(),
            b"MP3ext MP3ext X",
        ] {
            let path = std::env::temp_dir().join(format!(
                "alb-padding-boundary-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let mut bytes = b"ID3\x03\0\0\0\0\0\0".to_vec();
            bytes[9] = body.len() as u8;
            bytes.extend_from_slice(body);
            bytes.extend_from_slice(include_bytes!("../tests/fixtures/untagged.mp3"));
            std::fs::write(&path, &bytes).unwrap();
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            assert!(super::remove_empty_broken_frames(&mut file).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            drop(file);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn empty_broken_comment_removal_preserves_neighboring_bytes() {
        let path = std::env::temp_dir().join(format!(
            "alb-empty-frame-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let body = b"COMM\0\0\0\x02\0\0\0\0TIT2\0\0\0\x06\0\0\0Title";
        let mut source = b"ID3\x03\0\0\0\0\0\x1c".to_vec();
        assert_eq!(body.len(), 28);
        source.extend_from_slice(body);
        source.extend_from_slice(include_bytes!("../tests/fixtures/untagged.mp3"));
        std::fs::write(&path, &source).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let before = crate::media::payload(&mut file, crate::candidates::FileType::Mp3).unwrap();
        super::remove_empty_broken_frames(&mut file).unwrap();
        let mut expected = b"ID3\x03\0\0\0\0\0\x10".to_vec();
        expected.extend_from_slice(&body[12..]);
        expected.extend_from_slice(include_bytes!("../tests/fixtures/untagged.mp3"));
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        assert_eq!(
            crate::media::payload(&mut file, crate::candidates::FileType::Mp3).unwrap(),
            before
        );
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn comment_language_collision_does_not_drop_either_comment() {
        use lofty::id3::v2::{CommentFrame, Frame, Id3v2Tag};
        let mut tag = Id3v2Tag::new();
        for (language, text) in [(*b"und", "First"), (*b"   ", "Second")] {
            tag.insert(Frame::Comment(CommentFrame::new(
                lofty::TextEncoding::UTF8,
                language,
                "",
                text,
            )));
        }
        assert!(super::normalize_comment_languages(&mut tag).is_err());
        let contents: Vec<_> = tag
            .into_iter()
            .filter_map(|frame| {
                if let Frame::Comment(c) = frame {
                    Some(c.content.into_owned())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(contents, ["First", "Second"]);
    }
}
