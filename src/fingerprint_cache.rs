//! Disposable per-user cache. Never stores API keys or source paths.
use serde_json::Value;
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    base.filter(|p| p.is_absolute())
        .map(|p| p.join("alb/cache/fingerprints-v1"))
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub struct Cache {
    root: PathBuf,
}
impl Cache {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        // Refuse redirected ALB cache directories, including when clearing.
        for p in root.ancestors().take(3) {
            if let Ok(m) = fs::symlink_metadata(p) {
                if m.file_type().is_symlink() {
                    return Err(io::Error::other("cache directory is a symlink"));
                }
                if !m.is_dir() {
                    return Err(io::Error::other("cache directory is not a directory"));
                }
            }
        }
        Ok(Self { root })
    }
    pub fn key(path: &Path) -> Option<String> {
        let mut file = fs::File::open(path).ok()?;
        let before = crate::source::SourceStamp::from_file(&file).ok()?;
        let mut hash = blake3::Hasher::new();
        let mut buf = [0; 65536];
        loop {
            let n = file.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        if crate::source::SourceStamp::from_file(&file).ok()? != before {
            return None;
        }
        Some(hash.finalize().to_hex().to_string())
    }
    pub fn read(&self, key: &str) -> Option<Value> {
        let path = self.root.join(format!("{key}.json"));
        let m = fs::symlink_metadata(&path).ok()?;
        if !m.is_file() || m.len() > 2 * 1024 * 1024 {
            return None;
        }
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }
    pub fn write(&self, key: &str, value: &Value) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let dest = self.root.join(format!("{key}.json"));
        let tmp = self.root.join(format!(
            "{key}-{}-{}.tmp",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options.open(&tmp)?;
            file.write_all(&serde_json::to_vec(value)?)?;
            file.sync_all()?;
            drop(file);
            #[cfg(windows)]
            {
                match fs::remove_file(&dest) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            fs::rename(&tmp, dest)
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    pub fn clear(&self) -> io::Result<usize> {
        let entries = match fs::read_dir(&self.root) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut count = 0;
        for e in entries {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".json")
                && name.len() == 69
                && name.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
                && e.file_type()?.is_file()
            {
                fs::remove_file(e.path())?;
                count += 1;
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_records_are_misses_and_clear_preserves_unrelated_files() {
        let root = std::env::temp_dir().join(format!(
            "alb-cache-clear-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = Cache::new(root.clone()).unwrap();
        assert_eq!(cache.clear().unwrap(), 0);
        let key = "a".repeat(64);
        cache
            .write(&key, &serde_json::json!({"test": true}))
            .unwrap();
        assert!(cache.read(&key).is_some());
        fs::write(root.join(format!("{key}.json")), b"broken").unwrap();
        assert!(cache.read(&key).is_none());
        fs::write(root.join("config.toml"), b"retain").unwrap();
        assert_eq!(cache.clear().unwrap(), 1);
        assert_eq!(fs::read(root.join("config.toml")).unwrap(), b"retain");
        fs::remove_dir_all(root).unwrap();
    }
}
