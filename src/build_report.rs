//! Append-only, synced audit reports. A missing COMPLETE line means interruption.
use crate::{
    plan::{BuildPlan, PlanEntry},
    safe_fs::Directory,
};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Write},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct BuildReport {
    file: File,
}
impl BuildReport {
    pub fn start(output: &Directory, plan: &BuildPlan) -> io::Result<Self> {
        let (directory, _) = output.parent(std::path::Path::new("_ALB/report"), true)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let mut file = None;
        for attempt in 0..100 {
            let name = OsString::from(format!(
                "build-{timestamp}-{}-{attempt}.txt",
                std::process::id()
            ));
            match directory.create(&name) {
                Ok(created) => {
                    file = Some(created);
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        let mut file = file.ok_or_else(|| {
            io::Error::new(io::ErrorKind::AlreadyExists, "cannot reserve build report")
        })?;
        writeln!(
            file,
            "ALB BUILD REPORT v1\nSTARTED\nPaths are escaped native debug representations.\nOnly VERIFIED records confirm completed copies; a missing COMPLETE means interruption."
        )?;
        for root in &plan.input_roots {
            writeln!(file, "INPUT_ROOT {root:?}")?;
        }
        crate::plan::write_plan(&mut file, plan)?;
        for entry in &plan.entries {
            writeln!(
                file,
                "EVIDENCE source={:?} bytes={:?} blake3={}",
                entry.source,
                entry.source_stamp.as_ref().map(|s| s.len),
                entry
                    .digest
                    .map(|d| blake3::Hash::from(d).to_hex().to_string())
                    .unwrap_or_default()
            )?;
        }
        writeln!(
            file,
            "TIMESTAMP POLICY: source times are readable UTC dates with nanosecond precision. {} Unavailable means the source filesystem did not supply it. Duplicate sources retain individual timestamp records; output timestamps come from the representative.",
            crate::platform::CREATION_POLICY
        )?;
        for entry in &plan.entries {
            write_times(&mut file, entry)?;
        }
        file.sync_all()?;
        directory.sync()?;
        Ok(Self { file })
    }
    pub fn verified(&mut self, entry: &PlanEntry, duplicate: bool) -> io::Result<()> {
        write_times(&mut self.file, entry)?;
        write_output(&mut self.file, entry)?;
        writeln!(
            self.file,
            "{} source={:?} destination={:?}",
            if duplicate {
                "VERIFIED_DUPLICATE"
            } else {
                "VERIFIED_COPY"
            },
            entry.source,
            entry.destination
        )?;
        self.file.sync_all()
    }
    pub fn reused(&mut self, entry: &PlanEntry) -> io::Result<()> {
        write_times(&mut self.file, entry)?;
        write_output(&mut self.file, entry)?;
        writeln!(
            self.file,
            "VERIFIED_REUSE source={:?} destination={:?}",
            entry.source, entry.destination
        )?;
        self.file.sync_all()
    }
    pub fn failure(&mut self, entry: &PlanEntry, message: &str) -> io::Result<()> {
        writeln!(
            self.file,
            "FAILED source={:?} details={message:?}",
            entry.source
        )?;
        self.file.sync_all()
    }
    pub fn incomplete(&mut self, failures: usize) -> io::Result<()> {
        writeln!(self.file, "FINISHED_WITH_ERRORS failures={failures}")?;
        self.file.sync_all()
    }
    pub fn complete(&mut self, copied: usize, duplicates: usize, reused: usize) -> io::Result<()> {
        writeln!(
            self.file,
            "COMPLETE copied={copied} duplicates={duplicates} reused={reused}"
        )?;
        self.file.sync_all()
    }
}

fn write_times(file: &mut File, entry: &PlanEntry) -> io::Result<()> {
    writeln!(
        file,
        "SOURCE_TIMES source={:?} destination={:?} modified=\"{}\" created=\"{}\"",
        entry.source,
        entry.destination,
        crate::source::timestamp(entry.source_stamp.as_ref().map(|s| s.modified)),
        crate::source::timestamp(entry.source_stamp.as_ref().and_then(|s| s.created))
    )
}

fn write_output(file: &mut File, entry: &PlanEntry) -> io::Result<()> {
    if let Some((hash, bytes)) = entry.output_evidence {
        writeln!(
            file,
            "OUTPUT_EVIDENCE destination={:?} bytes={bytes} blake3={} metadata_update={:?}",
            entry.destination,
            blake3::Hash::from(hash).to_hex(),
            entry.metadata_update
        )?;
    }
    Ok(())
}
