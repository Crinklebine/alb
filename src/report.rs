//! Human-readable scan reports. All untrusted text is escaped for terminals.
use crate::{candidates::classify, catalog::TrackCatalog, discovery::Catalog};
use std::{
    collections::BTreeMap,
    io::{self, Write},
};

pub fn write_details(
    writer: &mut impl Write,
    catalog: &Catalog,
    inspected: &TrackCatalog,
) -> io::Result<()> {
    let tracks: BTreeMap<_, _> = inspected
        .tracks
        .iter()
        .map(|track| (&track.source_path, track))
        .collect();
    let errors: BTreeMap<_, _> = inspected
        .errors
        .iter()
        .map(|(path, error)| (path, error))
        .collect();
    for path in &catalog.files {
        if let Some(track) = tracks.get(path) {
            writeln!(writer, "{track}")?;
        } else {
            writeln!(writer, "{path:?}: {}", classify(path).group())?;
        }
        if let Some(error) = errors.get(path) {
            writeln!(writer, "  metadata/read error: {:?}", error.to_string())?;
        }
    }
    for error in &catalog.errors {
        writeln!(
            writer,
            "{:?}: discovery error: {:?}",
            error.path,
            error.source.to_string()
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn propagates_output_failure() {
        struct Fails;
        impl Write for Fails {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let catalog = Catalog {
            files: vec!["unknown".into()],
            ..Catalog::default()
        };
        assert_eq!(
            write_details(&mut Fails, &catalog, &TrackCatalog::default())
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
