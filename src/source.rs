//! Change indicators for read-only stages, not a filesystem race-proof guarantee.
use std::{fs, io, path::Path, time::SystemTime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStamp {
    pub len: u64,
    pub modified: SystemTime,
    pub created: Option<SystemTime>,
    identity: (u64, u64, i64, i64),
}

impl SourceStamp {
    pub fn from_file(file: &fs::File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() || crate::platform::is_link(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a regular file",
            ));
        }
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified()?,
            created: crate::platform::creation_time(&metadata),
            identity: crate::platform::identity(file)?,
        })
    }

    pub fn at(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if crate::platform::is_link(&metadata) || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a regular file",
            ));
        }
        Self::from_file(&crate::platform::open_snapshot(path)?)
    }
}

/// Human-readable UTC with nanosecond precision, including pre-1970 dates.
pub fn timestamp(value: Option<SystemTime>) -> String {
    let Some(value) = value else {
        return "unavailable".into();
    };
    let nanos = match value.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    let date = i64::try_from(nanos.div_euclid(1_000_000_000))
        .ok()
        .and_then(|seconds| {
            chrono::DateTime::from_timestamp(seconds, nanos.rem_euclid(1_000_000_000) as u32)
        });
    match date {
        Some(date) => date.format("%Y-%m-%d %H:%M:%S%.9f UTC").to_string(),
        None => "unavailable (outside supported calendar range)".into(),
    }
}

#[cfg(test)]
mod archival_tests {
    use super::*;
    #[test]
    fn leap_day_formats_in_utc_without_losing_fractional_seconds() {
        assert_eq!(
            timestamp(Some(
                SystemTime::UNIX_EPOCH + std::time::Duration::new(951827696, 123456700)
            )),
            "2000-02-29 12:34:56.123456700 UTC"
        );
    }
    #[test]
    fn archival_time_representation_is_exact_and_marks_absence() {
        assert_eq!(timestamp(None), "unavailable");
        assert_eq!(
            timestamp(Some(SystemTime::UNIX_EPOCH)),
            "1970-01-01 00:00:00.000000000 UTC"
        );
        assert_eq!(
            timestamp(Some(
                SystemTime::UNIX_EPOCH - std::time::Duration::from_nanos(100)
            )),
            "1969-12-31 23:59:59.999999900 UTC"
        );
    }
}
