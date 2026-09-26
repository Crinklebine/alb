//! Generic sorting data; no decoder or codec-specific properties.
use crate::{
    candidates::{FileType, classify},
    source::SourceStamp,
};
use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Debug)]
pub struct Track {
    pub metadata_notes: Vec<String>,
    pub fingerprinted: bool,
    pub metadata_update: Option<crate::tagging::MetadataUpdate>,
    pub source_path: PathBuf,
    pub file_type: FileType,
    pub source_stamp: Option<SourceStamp>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration: Option<Duration>,
}

// Debug escaping keeps control characters in paths/tags from affecting terminals.
impl fmt::Display for Track {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}: {} metadata; title={:?}, artist={:?}, album_artist={:?}, album={:?}, track={:?}, disc={:?}, duration={:?}",
            self.source_path,
            self.file_type.group(),
            self.title,
            self.artist,
            self.album_artist,
            self.album,
            self.track_number,
            self.disc_number,
            self.duration,
        )
    }
}

impl Track {
    pub fn empty(path: &Path) -> Self {
        Self {
            metadata_notes: Vec::new(),
            fingerprinted: false,
            metadata_update: None,
            source_path: path.to_owned(),
            file_type: classify(path),
            source_stamp: None,
            title: None,
            artist: None,
            album_artist: None,
            album: None,
            track_number: None,
            disc_number: None,
            duration: None,
        }
    }
}

/// Every discovered regular file has a track, even if metadata is absent/broken.
#[derive(Debug, Default)]
pub struct TrackCatalog {
    pub tracks: Vec<Track>,
    pub errors: Vec<(PathBuf, String)>,
}
