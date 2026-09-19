//! V1 output types are determined only by the final filename extension.
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileType {
    Flac,
    M4a,
    Mp3,
    Ogg,
    Wav,
    Unknown,
}
impl FileType {
    pub fn group(self) -> &'static str {
        match self {
            Self::Flac => "FLAC",
            Self::M4a => "M4A",
            Self::Mp3 => "MP3",
            Self::Ogg => "OGG",
            Self::Wav => "WAV",
            Self::Unknown => "UNKNOWN",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Wav => "wav",
            Self::Unknown => "",
        }
    }
}
pub fn classify(path: &Path) -> FileType {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "flac" => FileType::Flac,
        "m4a" => FileType::M4a,
        "mp3" => FileType::Mp3,
        "ogg" => FileType::Ogg,
        "wav" => FileType::Wav,
        _ => FileType::Unknown,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_five_extensions_are_supported_case_insensitively() {
        for (name, kind) in [
            ("x.FLAC", FileType::Flac),
            ("x.m4A", FileType::M4a),
            ("x.Mp3", FileType::Mp3),
            ("x.OGG", FileType::Ogg),
            ("x.Wav", FileType::Wav),
        ] {
            assert_eq!(classify(Path::new(name)), kind);
        }
        for name in [
            "x.aac",
            "x.m4b",
            "x.mp4",
            "x.oga",
            "x.opus",
            "x.wave",
            "x.aiff",
            "x.flac.bak",
            ".flac",
            "README",
            "x.flac ",
        ] {
            assert_eq!(classify(Path::new(name)), FileType::Unknown, "{name}");
        }
    }
    #[cfg(unix)]
    #[test]
    fn non_unicode_paths_do_not_change_classification() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        assert_eq!(
            classify(Path::new(&OsString::from_vec(b"x\xff.FLAC".to_vec()))),
            FileType::Flac
        );
        assert_eq!(
            classify(Path::new(&OsString::from_vec(b"x.flac\xff".to_vec()))),
            FileType::Unknown
        );
    }
}
