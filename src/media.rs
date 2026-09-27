//! Bounded format identification and structural checks, not an audio decoder.
use crate::candidates::FileType;
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};
const TAG_LIMIT: u64 = 16 * 1024 * 1024;
fn bad(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn at(file: &mut File, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(bytes)
}
pub fn id3_end(file: &mut File) -> io::Result<u64> {
    let len = file.metadata()?.len();
    let mut offset = 0;
    for _ in 0..32 {
        if len.saturating_sub(offset) < 3 {
            return Ok(offset);
        }
        let mut prefix = [0; 3];
        at(file, offset, &mut prefix)?;
        if &prefix != b"ID3" {
            return Ok(offset);
        }
        if len.saturating_sub(offset) < 10 {
            return Err(bad("truncated ID3 header"));
        }
        let mut h = [0; 10];
        at(file, offset, &mut h)?;
        if !(2..=4).contains(&h[3]) || h[6..].iter().any(|v| v & 128 != 0) {
            return Err(bad("invalid ID3 header"));
        }
        let size = h[6..].iter().fold(0u64, |a, b| (a << 7) | *b as u64);
        if size > TAG_LIMIT {
            return Err(bad("ID3 block exceeds 16 MiB processing limit"));
        }
        offset += 10 + size + if h[3] == 4 && h[5] & 16 != 0 { 10 } else { 0 };
        if offset > 32 * 1024 * 1024 {
            return Err(bad("leading ID3 metadata exceeds 32 MiB processing limit"));
        }
        if offset > len {
            return Err(bad("truncated ID3 block"));
        }
    }
    Err(bad("too many consecutive ID3 blocks"))
}
// Ambiguous unknown extensions need two consistent, complete MPEG frames.
// Known audio extensions and explicit container/tag signatures keep their existing path.
fn consistent_mpeg_frames(file: &mut File, len: u64) -> io::Result<bool> {
    fn header(h: [u8; 4]) -> Option<(u64, u8, u8, u8)> {
        let version = (h[1] >> 3) & 3;
        let layer = (h[1] >> 1) & 3;
        let index = (h[2] >> 4) as usize;
        let rate = (h[2] >> 2) & 3;
        if h[0] != 255
            || h[1] & 224 != 224
            || version == 1
            || layer == 0
            || index == 0
            || index == 15
            || rate == 3
        {
            return None;
        }
        let rates = match (version == 3, layer) {
            (true, 3) => [
                32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
            ],
            (true, 2) => [
                32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
            ],
            (true, 1) => [
                32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ],
            (false, 3) => [
                32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
            ],
            _ => [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        };
        let sample_rate = [44100, 48000, 32000][rate as usize]
            / match version {
                3 => 1,
                2 => 2,
                _ => 4,
            };
        let bitrate = rates[index - 1] * 1000u64;
        let padding = ((h[2] >> 1) & 1) as u64;
        let length = if layer == 3 {
            (12 * bitrate / sample_rate + padding) * 4
        } else {
            (if layer == 1 && version != 3 { 72 } else { 144 }) * bitrate / sample_rate + padding
        };
        Some((length, version, layer, rate))
    }
    file.rewind()?;
    let mut h = [0; 4];
    file.read_exact(&mut h)?;
    let Some(first) = header(h) else {
        return Ok(false);
    };
    if first.0 + 4 > len {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(first.0))?;
    file.read_exact(&mut h)?;
    Ok(header(h).is_some_and(|next| {
        (first.1, first.2, first.3) == (next.1, next.2, next.3) && first.0 + next.0 <= len
    }))
}

/// A zero-file check stops at the first nonzero byte, so ordinary files are cheap.
pub fn identify(file: &mut File, hinted: FileType) -> io::Result<FileType> {
    let len = file.metadata()?.len();
    if len == 0 {
        return if hinted == FileType::Unknown {
            Ok(FileType::Unknown)
        } else {
            Err(bad("empty file"))
        };
    }
    if hinted == FileType::Unknown {
        let mut prefix = [0; 12];
        file.rewind()?;
        let n = file.read(&mut prefix)?;
        if n < 4
            || !(&prefix[..3] == b"ID3"
                || &prefix[..4] == b"fLaC"
                || &prefix[..4] == b"OggS"
                || (n >= 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"WAVE")
                || (n >= 8 && &prefix[4..8] == b"ftyp")
                || (prefix[0] == 255
                    && prefix[1] & 224 == 224
                    && consistent_mpeg_frames(file, len)?))
        {
            file.rewind()?;
            return Ok(FileType::Unknown);
        }
    }
    file.rewind()?;
    let mut buf = [0; 65536];
    let mut nonzero = false;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if buf[..n].iter().any(|b| *b != 0) {
            nonzero = true;
            break;
        }
    }
    if !nonzero {
        return Err(bad(
            "file contains only zero bytes; no recoverable audio or metadata",
        ));
    }
    let mut h = [0; 12];
    file.rewind()?;
    let n = file.read(&mut h)?;
    let kind = if n >= 4 && &h[..4] == b"fLaC" {
        FileType::Flac
    } else if n >= 12 && &h[..4] == b"RIFF" && &h[8..12] == b"WAVE" {
        FileType::Wav
    } else if n >= 4 && &h[..4] == b"OggS" {
        FileType::Ogg
    } else if n >= 8 && &h[4..8] == b"ftyp" {
        FileType::M4a
    } else {
        let offset = id3_end(file)?;
        file.seek(SeekFrom::Start(offset))?;
        let probe = lofty::probe::Probe::new(&mut *file)
            .options(lofty::config::ParseOptions::new().max_junk_bytes(1024 * 1024))
            .guess_file_type()?;
        match probe.file_type() {
            Some(lofty::file::FileType::Mpeg) => FileType::Mp3,
            Some(lofty::file::FileType::Wav) => FileType::Wav,
            Some(lofty::file::FileType::Flac) => FileType::Flac,
            Some(lofty::file::FileType::Mp4) => FileType::M4a,
            Some(
                lofty::file::FileType::Vorbis
                | lofty::file::FileType::Opus
                | lofty::file::FileType::Speex,
            ) => FileType::Ogg,
            Some(_) => return Err(bad("unsupported audio container")),
            None if hinted == FileType::Unknown => FileType::Unknown,
            None => return Err(bad("unrecognized or damaged audio container")),
        }
    };
    let kind = if kind == FileType::M4a && mp4_has_video(file)? {
        FileType::Unknown
    } else {
        kind
    };
    file.rewind()?;
    Ok(kind)
}
fn range(
    file: &mut File,
    hash: &mut blake3::Hasher,
    start: u64,
    len: u64,
    calculate: bool,
) -> io::Result<()> {
    if !calculate {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut left = len;
    let mut buf = [0; 65536];
    while left > 0 {
        let wanted = left.min(buf.len() as u64) as usize;
        file.read_exact(&mut buf[..wanted])?;
        hash.update(&buf[..wanted]);
        left -= wanted as u64;
    }
    Ok(())
}
/// Hash encoded audio regions, excluding metadata. Also check declared container bounds.
pub fn structure(file: &mut File, kind: FileType) -> io::Result<()> {
    process(file, kind, false).map(|_| ())
}
pub fn payload(file: &mut File, kind: FileType) -> io::Result<[u8; 32]> {
    process(file, kind, true)
}
fn process(file: &mut File, kind: FileType, calculate: bool) -> io::Result<[u8; 32]> {
    let len = file.metadata()?.len();
    let mut hash = blake3::Hasher::new();
    let mut audio = false;
    match kind {
        FileType::Mp3 => {
            let start = id3_end(file)?;
            let mut end = len;
            if end >= 128 {
                let mut marker = [0; 3];
                at(file, end - 128, &mut marker)?;
                if &marker == b"TAG" {
                    end -= 128;
                }
            }
            if end < start {
                return Err(bad("overlapping MPEG metadata regions"));
            }
            if end >= 32 {
                let mut footer = [0; 32];
                at(file, end - 32, &mut footer)?;
                if &footer[..8] == b"APETAGEX" {
                    let size = u32::from_le_bytes(footer[12..16].try_into().unwrap()) as u64;
                    if size < 32 || size > end - start {
                        return Err(bad("invalid APE tag size"));
                    }
                    end -= size;
                    if end >= 32 {
                        let mut marker = [0; 8];
                        at(file, end - 32, &mut marker)?;
                        if &marker == b"APETAGEX" {
                            end -= 32;
                        }
                    }
                }
            }
            if end <= start {
                return Err(bad("no MPEG audio payload"));
            }
            range(file, &mut hash, start, end - start, calculate)?;
            audio = true;
        }
        FileType::Flac => {
            let mut magic = [0; 4];
            at(file, 0, &mut magic)?;
            if &magic != b"fLaC" {
                return Err(bad("missing FLAC stream marker"));
            }
            let mut pos = 4;
            loop {
                let mut h = [0; 4];
                at(file, pos, &mut h)?;
                let size = ((h[1] as u64) << 16) | ((h[2] as u64) << 8) | h[3] as u64;
                if pos + 4 + size > len {
                    return Err(bad("truncated FLAC metadata block"));
                }
                if h[0] & 127 == 0 {
                    range(file, &mut hash, pos + 4, size, calculate)?;
                } // STREAMINFO describes the encoded audio.
                pos += 4 + size;
                if h[0] & 128 != 0 {
                    break;
                }
            }
            if pos >= len {
                return Err(bad("missing FLAC audio frames"));
            }
            range(file, &mut hash, pos, len - pos, calculate)?;
            audio = true;
        }
        FileType::Wav => {
            let layout = wav_layout(file)?;
            let end = layout.end;
            let mut pos = layout.start + 12;
            while pos < end {
                if end - pos < 8 {
                    return Err(bad("truncated WAV chunk header"));
                }
                let mut h = [0; 8];
                at(file, pos, &mut h)?;
                let size = u32::from_le_bytes(h[4..].try_into().unwrap()) as u64;
                if pos + 8 + size > end {
                    return Err(bad(&format!(
                        "truncated WAV chunk at {pos}: size {size}, RIFF end {end}, file length {len}"
                    )));
                }
                if &h[..4] == b"data" || &h[..4] == b"fmt " {
                    hash.update(&h[..4]);
                    range(file, &mut hash, pos + 8, size, calculate)?;
                    if &h[..4] == b"data" && size > 0 {
                        audio = true;
                    }
                }
                pos += 8 + size + (size % 2);
            }
        }
        FileType::M4a => {
            let mut pos = 0;
            while pos < len {
                if len - pos < 8 {
                    return Err(bad("truncated MP4 atom header"));
                }
                let mut h = [0; 8];
                at(file, pos, &mut h)?;
                let mut size = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
                let mut header = 8;
                if size == 1 {
                    let mut large = [0; 8];
                    at(file, pos + 8, &mut large)?;
                    size = u64::from_be_bytes(large);
                    header = 16;
                } else if size == 0 {
                    size = len - pos;
                }
                if size < header || size > len - pos {
                    return Err(bad("truncated or invalid MP4 atom"));
                }
                if &h[4..] == b"mdat" {
                    range(file, &mut hash, pos + header, size - header, calculate)?;
                    audio |= size > header;
                }
                pos += size;
            }
        }
        FileType::Ogg => {
            // Hash reconstructed packets; comment packets are the only packets omitted.
            let mut pos = 0;
            let mut packet = Vec::new();
            let mut index = 0;
            let mut audio_start = 3;
            let mut serial = None;
            while pos < len {
                let mut h = [0; 27];
                at(file, pos, &mut h)?;
                if &h[..4] != b"OggS" || h[4] != 0 {
                    return Err(bad("invalid Ogg page"));
                }
                let stream = u32::from_le_bytes(h[14..18].try_into().unwrap());
                if serial.is_some_and(|v| v != stream) {
                    return Err(bad("chained/multiplexed Ogg requires manual review"));
                }
                serial = Some(stream);
                let count = h[26] as usize;
                let mut sizes = vec![0; count];
                at(file, pos + 27, &mut sizes)?;
                let total: u64 = sizes.iter().map(|b| *b as u64).sum();
                if pos + 27 + count as u64 + total > len {
                    return Err(bad("truncated Ogg page"));
                }
                let mut data = pos + 27 + count as u64;
                for size in sizes {
                    if packet.len() + size as usize > 16 * 1024 * 1024 {
                        return Err(bad("oversized Ogg packet"));
                    }
                    let start = packet.len();
                    packet.resize(start + size as usize, 0);
                    at(file, data, &mut packet[start..])?;
                    data += size as u64;
                    if size < 255 {
                        if index == 0 {
                            audio_start = if packet.starts_with(b"\x01vorbis") {
                                3
                            } else if packet.starts_with(b"OpusHead")
                                || packet.starts_with(b"Speex   ")
                            {
                                2
                            } else {
                                return Err(bad("unsupported Ogg codec"));
                            };
                        }
                        if index != 1 {
                            hash.update(&(packet.len() as u64).to_le_bytes());
                            hash.update(&packet);
                            if index >= audio_start && !packet.is_empty() {
                                audio = true;
                            }
                        }
                        index += 1;
                        packet.clear();
                    }
                }
                pos = data;
            }
            if !packet.is_empty() {
                return Err(bad("truncated Ogg packet"));
            }
        }
        FileType::Unknown => return Err(bad("unsupported audio format")),
    }
    file.rewind()?;
    if !audio {
        return Err(bad("missing audio payload"));
    }
    Ok(*hash.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            for n in 0.. {
                let path =
                    std::env::temp_dir().join(format!("alb-media-{}-{n}", std::process::id()));
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                {
                    use std::io::Write;
                    f.write_all(bytes).unwrap();
                    return Self(path);
                }
            }
            unreachable!()
        }
        fn file(&self) -> File {
            File::open(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    #[test]
    fn detects_containers_independently_of_extension_hints() {
        for (kind, bytes) in [
            (
                FileType::Flac,
                include_bytes!("../tests/fixtures/tone.flac").as_slice(),
            ),
            (
                FileType::M4a,
                include_bytes!("../tests/fixtures/tone.m4a").as_slice(),
            ),
            (
                FileType::Mp3,
                include_bytes!("../tests/fixtures/tone.mp3").as_slice(),
            ),
            (
                FileType::Wav,
                include_bytes!("../tests/fixtures/tone.wav").as_slice(),
            ),
            (
                FileType::Ogg,
                include_bytes!("../tests/fixtures/tone.ogg").as_slice(),
            ),
        ] {
            let f = Fixture::new(bytes);
            assert_eq!(identify(&mut f.file(), FileType::Unknown).unwrap(), kind);
            assert_eq!(identify(&mut f.file(), FileType::Mp3).unwrap(), kind);
            assert!(payload(&mut f.file(), kind).is_ok());
        }
    }
    #[test]
    fn rejects_empty_zeroed_and_declared_truncated_audio() {
        for bytes in [vec![], vec![0; 128 * 1024]] {
            let f = Fixture::new(&bytes);
            assert!(identify(&mut f.file(), FileType::Mp3).is_err());
        }
        for (kind, bytes) in [
            (
                FileType::Wav,
                include_bytes!("../tests/fixtures/tone.wav").as_slice(),
            ),
            (
                FileType::M4a,
                include_bytes!("../tests/fixtures/tone.m4a").as_slice(),
            ),
            (
                FileType::Ogg,
                include_bytes!("../tests/fixtures/tone.ogg").as_slice(),
            ),
        ] {
            let f = Fixture::new(&bytes[..bytes.len() / 2]);
            assert!(payload(&mut f.file(), kind).is_err(), "{kind:?}");
        }
        let f = Fixture::new(b"ID3\x03\0\0\0\0\x7f\x7f");
        assert!(identify(&mut f.file(), FileType::Mp3).is_err());
        let f = Fixture::new(b"fLaC\x80\x01\0\0");
        assert!(payload(&mut f.file(), FileType::Flac).is_err());
    }
    #[test]
    fn unknown_empty_and_non_audio_files_remain_unknown() {
        for bytes in [b"".as_slice(), b"ordinary notes"] {
            let f = Fixture::new(bytes);
            assert_eq!(
                identify(&mut f.file(), FileType::Unknown).unwrap(),
                FileType::Unknown
            );
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WavLayout {
    pub start: u64,
    pub end: u64,
    pub legacy_tag: bool,
}
/// WAV may have external ID3 tags; those bytes are not RIFF chunks or audio.
pub fn wav_layout(reader: &mut (impl Read + Seek)) -> io::Result<WavLayout> {
    let len = reader.seek(SeekFrom::End(0))?;
    let mut start = 0;
    for _ in 0..32 {
        if len.saturating_sub(start) < 12 {
            return Err(bad("truncated WAV header"));
        }
        reader.seek(SeekFrom::Start(start))?;
        let mut h = [0; 12];
        reader.read_exact(&mut h)?;
        if &h[..3] != b"ID3" {
            if &h[..4] != b"RIFF" || &h[8..] != b"WAVE" {
                return Err(bad("missing RIFF/WAVE header"));
            }
            let mut end = len;
            let mut legacy_tag = false;
            if len >= start + 12 + 128 {
                reader.seek(SeekFrom::Start(len - 128))?;
                let mut marker = [0; 3];
                reader.read_exact(&mut marker)?;
                if &marker == b"TAG" {
                    end -= 128;
                    legacy_tag = true;
                }
            }
            let declared = start + u32::from_le_bytes(h[4..8].try_into().unwrap()) as u64 + 8;
            if declared > end || declared < start + 12 {
                return Err(bad(&format!(
                    "truncated RIFF/WAV container: declares end at byte {declared}, available container ends at {end}"
                )));
            }
            return Ok(WavLayout {
                start,
                end,
                legacy_tag,
            });
        }
        if !(2..=4).contains(&h[3]) || h[6..10].iter().any(|b| b & 128 != 0) {
            return Err(bad("invalid leading ID3 header"));
        }
        let size = h[6..10].iter().fold(0u64, |n, b| (n << 7) | *b as u64);
        if size > TAG_LIMIT {
            return Err(bad("oversized leading ID3 tag"));
        }
        start += 10 + size + if h[3] == 4 && h[5] & 16 != 0 { 10 } else { 0 };
        if start > 32 * 1024 * 1024 {
            return Err(bad("oversized leading ID3 region"));
        }
    }
    Err(bad("too many leading ID3 blocks"))
}
/// Give a container reader its own coordinate system, excluding external tags.
pub struct Window<'a, R> {
    reader: &'a mut R,
    start: u64,
    len: u64,
    pos: u64,
}
impl<'a, R: Read + Seek> Window<'a, R> {
    pub fn new(reader: &'a mut R, start: u64, end: u64) -> io::Result<Self> {
        let len = end
            .checked_sub(start)
            .ok_or_else(|| bad("invalid container window"))?;
        reader.seek(SeekFrom::Start(start))?;
        Ok(Self {
            reader,
            start,
            len,
            pos: 0,
        })
    }
}
impl<R: Read + Seek> Read for Window<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = (buf.len() as u64).min(self.len.saturating_sub(self.pos)) as usize;
        let n = self.reader.read(&mut buf[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}
impl<R: Read + Seek> Seek for Window<'_, R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.pos as i128 + n as i128,
            SeekFrom::End(n) => self.len as i128 + n as i128,
        };
        if next < 0 || next > u64::MAX as i128 {
            return Err(bad("invalid container seek"));
        }
        self.pos = next as u64;
        let absolute = self
            .start
            .checked_add(self.pos)
            .ok_or_else(|| bad("container seek overflow"))?;
        self.reader.seek(SeekFrom::Start(absolute))?;
        Ok(self.pos)
    }
}

// Follow only movie/track/media boxes. Artwork in metadata is not a video track.
fn mp4_has_video(file: &mut File) -> io::Result<bool> {
    fn boxes(
        file: &mut File,
        start: u64,
        end: u64,
        depth: u8,
        budget: &mut usize,
    ) -> io::Result<bool> {
        let mut pos = start;
        while end.saturating_sub(pos) >= 8 {
            if *budget == 0 {
                return Ok(false);
            }
            *budget -= 1;
            file.seek(SeekFrom::Start(pos))?;
            let mut h = [0; 8];
            file.read_exact(&mut h)?;
            let short = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
            let (size, header) = if short == 1 {
                let mut n = [0; 8];
                file.read_exact(&mut n)?;
                (u64::from_be_bytes(n), 16)
            } else if short == 0 {
                (end - pos, 8)
            } else {
                (short, 8)
            };
            let Some(next) = pos
                .checked_add(size)
                .filter(|n| *n <= end && size >= header)
            else {
                return Ok(false);
            };
            if depth == 3 && &h[4..] == b"hdlr" && size >= header + 12 {
                file.seek(SeekFrom::Start(pos + header + 8))?;
                let mut handler = [0; 4];
                file.read_exact(&mut handler)?;
                if &handler == b"vide" {
                    return Ok(true);
                }
            }
            let child = match depth {
                0 => b"moov",
                1 => b"trak",
                2 => b"mdia",
                _ => b"----",
            };
            if depth < 3 && &h[4..] == child && boxes(file, pos + header, next, depth + 1, budget)?
            {
                return Ok(true);
            }
            pos = next;
        }
        Ok(false)
    }
    boxes(file, 0, file.metadata()?.len(), 0, &mut 100000)
}

#[cfg(test)]
mod ambiguous_mpeg_tests {
    use super::*;
    #[test]
    fn frame_evidence_supports_versions_layers_and_crc_without_utf16_blacklist() {
        let path = std::env::temp_dir().join(format!(
            "alb-mpeg-evidence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (header, length) in [
            ([255, 251, 144, 0], 417),
            ([255, 243, 128, 0], 208),
            ([255, 227, 128, 0], 417),
            ([255, 254, 160, 0], 348),
            ([255, 253, 160, 0], 626),
        ] {
            let mut data = vec![0; length * 2];
            data[..4].copy_from_slice(&header);
            data[length..length + 4].copy_from_slice(&header);
            std::fs::write(&path, &data).unwrap();
            let mut file = File::open(&path).unwrap();
            assert!(consistent_mpeg_frames(&mut file, data.len() as u64).unwrap());
            drop(file);
            data.truncate(length + 3);
            std::fs::write(&path, &data).unwrap();
            let mut file = File::open(&path).unwrap();
            assert!(!consistent_mpeg_frames(&mut file, data.len() as u64).unwrap());
        }
        std::fs::remove_file(path).unwrap();
    }
}
