//! Modify only newly created output handles, never sources or existing outputs.
use lofty::{
    config::{ParseOptions, ParsingMode, WriteOptions},
    file::{AudioFile, FileType, TaggedFileExt},
    probe::Probe,
    tag::{Accessor, Tag, TagExt, TagType},
};
use std::{
    fs::File,
    io::{self, Seek},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataUpdate {
    pub artist: Option<String>,
    pub title: Option<String>,
}

pub fn apply(
    file: &mut File,
    update: &MetadataUpdate,
    kind: crate::candidates::FileType,
) -> io::Result<()> {
    let mut operation = || -> Result<(), Box<dyn std::error::Error>> {
        file.rewind()?;
        // Retain artwork and unrelated tags. Write just the primary tag.
        let mut tagged = probe(file, kind)?
            .options(ParseOptions::new().read_cover_art(true))
            .read()?;
        if tagged.file_type() == FileType::Mpeg {
            // Native ID3 editing avoids generic conversion's randomized frame order
            // and retains frames that have no generic Tag equivalent.
            file.rewind()?;
            let mut mpeg = lofty::mpeg::MpegFile::read_from(&mut *file, ParseOptions::new())?;
            if mpeg.id3v2().is_none() {
                mpeg.set_id3v2(lofty::id3::v2::Id3v2Tag::new());
            }
            let tag = mpeg.id3v2_mut().ok_or("missing ID3 tag")?;
            if let Some(artist) = &update.artist {
                tag.set_artist(artist.clone());
            }
            if let Some(title) = &update.title {
                tag.set_title(title.clone());
            }
            file.rewind()?;
            tag.save_to(
                &mut *file,
                WriteOptions::new()
                    .parse_options(ParseOptions::new().max_junk_bytes(16 * 1024 * 1024)),
            )?;
        } else {
            let kind = if tagged.file_type() == FileType::Wav {
                TagType::RiffInfo
            } else {
                tagged.primary_tag_type()
            };
            if tagged.tag(kind).is_none() {
                tagged.insert_tag(Tag::new(kind));
            }
            let tag = tagged.tag_mut(kind).ok_or("no writable primary tag")?;
            if let Some(artist) = &update.artist {
                tag.set_artist(artist.clone());
            }
            if let Some(title) = &update.title {
                tag.set_title(title.clone());
            }
            file.rewind()?;
            tag.save_to(
                &mut *file,
                WriteOptions::new()
                    .parse_options(ParseOptions::new().max_junk_bytes(16 * 1024 * 1024)),
            )?;
        }
        file.sync_all()?;
        file.rewind()?;
        let actual = probe(file, kind)?
            .options(ParseOptions::new().parsing_mode(ParsingMode::BestAttempt))
            .read()?;
        let kind = if actual.file_type() == FileType::Wav {
            TagType::RiffInfo
        } else {
            actual.primary_tag_type()
        };
        file.rewind()?;
        let blocks = if actual.file_type() == FileType::Mpeg {
            crate::inspection::mp3_blocks(file)?
        } else {
            Vec::new()
        };
        let tag = blocks
            .first()
            .or_else(|| actual.tag(kind))
            .ok_or("updated tag missing")?;
        if update
            .artist
            .as_deref()
            .is_some_and(|s| tag.artist().as_deref() != Some(s))
            || update
                .title
                .as_deref()
                .is_some_and(|s| tag.title().as_deref() != Some(s))
        {
            return Err("recovered metadata did not round-trip".into());
        }
        Ok(())
    };
    operation().map_err(|e| io::Error::other(format!("metadata update failed: {e}")))
}

fn probe(file: &mut File, kind: crate::candidates::FileType) -> io::Result<Probe<&mut File>> {
    use crate::candidates::FileType as Kind;
    let ty = match kind {
        Kind::Mp3 => FileType::Mpeg,
        Kind::M4a => FileType::Mp4,
        Kind::Flac => FileType::Flac,
        Kind::Wav => FileType::Wav,
        _ => return Probe::new(file).guess_file_type(),
    };
    Ok(Probe::with_file_type(file, ty))
}
