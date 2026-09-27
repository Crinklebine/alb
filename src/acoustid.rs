//! Optional, sequential identification. Never expose keys, subprocess output or HTTP errors.
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const INTERVAL: Duration = Duration::from_millis(350);
const LIMIT: u64 = 2 * 1024 * 1024;
pub const REJECTED_WARNING: &str = "warning: AcoustID API key was rejected; disabling AcoustID lookup for the remainder of this run";

#[derive(PartialEq, Eq)]
pub struct ApiKey(String);
impl ApiKey {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Identification {
    pub artist: String,
    pub title: String,
}
struct Fingerprint {
    duration: u64,
    fingerprint: String,
}
struct Response {
    status: u16,
    body: Vec<u8>,
}
trait Backend {
    fn fingerprint(&mut self, path: &Path) -> Option<Fingerprint>;
    fn request(&mut self, key: &ApiKey, fingerprint: &Fingerprint) -> Option<Response>;
    fn now(&self) -> Duration;
    fn sleep(&mut self, duration: Duration);
}
struct Live {
    agent: ureq::Agent,
    origin: Instant,
}
impl Live {
    fn new() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(20)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
            origin: Instant::now(),
        }
    }
}
impl Backend for Live {
    fn fingerprint(&mut self, path: &Path) -> Option<Fingerprint> {
        fingerprint(path)
    }
    fn request(&mut self, key: &ApiKey, fp: &Fingerprint) -> Option<Response> {
        // POST keeps the key out of URLs. No redirects, retries or raw error logging.
        let duration = fp.duration.to_string();
        let mut response = self
            .agent
            .post("https://api.acoustid.org/v2/lookup")
            .send_form([
                ("client", key.0.as_str()),
                ("duration", &duration),
                ("fingerprint", &fp.fingerprint),
                ("meta", "recordings"),
                ("format", "json"),
            ])
            .ok()?;
        let status = response.status().as_u16();
        if matches!(status, 401 | 403) {
            return Some(Response {
                status,
                body: Vec::new(),
            });
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(LIMIT)
            .read_to_vec()
            .ok()?;
        Some(Response { status, body })
    }
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
    fn sleep(&mut self, duration: Duration) {
        thread::sleep(duration);
    }
}
fn parse_fingerprint(bytes: &[u8]) -> Option<Fingerprint> {
    let data: Value = serde_json::from_slice(bytes).ok()?;
    let duration = data.get("duration")?.as_f64()?;
    let fingerprint = data.get("fingerprint")?.as_str()?.trim();
    if !duration.is_finite()
        || !(1.0..=u32::MAX as f64).contains(&duration)
        || fingerprint.is_empty()
        || !fingerprint
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-=".contains(&b))
    {
        return None;
    }
    Some(Fingerprint {
        duration: duration.round() as u64,
        fingerprint: fingerprint.into(),
    })
}
fn fingerprint(path: &Path) -> Option<Fingerprint> {
    // Absolute paths cannot be interpreted as fpcalc switches. No shell is used.
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut command = Command::new("fpcalc");
    command.arg("-json").arg(path);
    run_fingerprint(&mut command)
}
fn run_fingerprint(command: &mut Command) -> Option<Fingerprint> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    });
    let start = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < Duration::from_secs(60) => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader.join().ok()??;
    if !success || bytes.len() as u64 > LIMIT {
        return None;
    }
    parse_fingerprint(&bytes)
}

pub trait Lookup {
    fn identify(&mut self, path: &Path) -> Option<Identification>;
}
struct Session<B> {
    key: ApiKey,
    backend: B,
    disabled: bool,
    last_start: Option<Duration>,
    warning_pending: bool,
    attempts: usize,
    successes: usize,
    offline: bool,
    cache_hits: usize,
    cache_matches: usize,
    cache: Option<crate::fingerprint_cache::Cache>,
}
pub struct AcoustId {
    session: Session<Live>,
}
impl AcoustId {
    #[cfg(test)]
    pub fn new(key: ApiKey) -> Self {
        Self::configured(Some(key), false)
    }
    pub fn configured(key: Option<ApiKey>, no_cache: bool) -> Self {
        let offline = key.is_none();
        let mut session = Session::with_backend(
            key.unwrap_or_else(|| ApiKey::new(String::new())),
            Live::new(),
        );
        session.offline = offline;
        if !no_cache {
            session.cache = crate::fingerprint_cache::directory()
                .and_then(|root| crate::fingerprint_cache::Cache::new(root).ok());
        }
        Self { session }
    }
    pub fn summary(&self) -> String {
        let attempts = self.session.attempts;
        let successes = self.session.successes;
        let new_results = if attempts == 0 {
            "0 attempts".to_owned()
        } else {
            let percentage = successes as f64 * 100.0 / attempts as f64;
            format!("{attempts} attempts, {successes} matched ({percentage:.1}%)")
        };
        let cached_matches = self.session.cache_matches;
        let cached_unmatched = self.session.cache_hits.saturating_sub(cached_matches);
        format!(
            "Fingerprinting: new: {new_results}; cached: {cached_matches} matched, {cached_unmatched} unmatched."
        )
    }

    pub fn take_warning(&mut self) -> Option<&'static str> {
        self.session.take_warning()
    }
}
impl Lookup for AcoustId {
    fn identify(&mut self, path: &Path) -> Option<Identification> {
        self.session.identify(path)
    }
}
impl<B: Backend> Session<B> {
    fn with_backend(key: ApiKey, backend: B) -> Self {
        Self {
            key,
            backend,
            disabled: false,
            last_start: None,
            warning_pending: false,
            attempts: 0,
            successes: 0,
            offline: false,
            cache_hits: 0,
            cache_matches: 0,
            cache: None,
        }
    }
    pub fn take_warning(&mut self) -> Option<&'static str> {
        std::mem::take(&mut self.warning_pending).then_some(REJECTED_WARNING)
    }
}
impl<B: Backend> Lookup for Session<B> {
    fn identify(&mut self, path: &Path) -> Option<Identification> {
        if self.disabled {
            return None;
        }
        let source_stamp = crate::source::SourceStamp::at(path).ok();
        let cache_key = self
            .cache
            .as_ref()
            .and_then(|_| crate::fingerprint_cache::Cache::key(path));
        let cached = self
            .cache
            .as_ref()
            .zip(cache_key.as_ref())
            .and_then(|(cache, key)| cache.read(key));
        let now = crate::fingerprint_cache::now();
        if let Some(record) = &cached {
            let ttl = if record["match"].is_object() {
                90 * 86400
            } else {
                7 * 86400
            };
            if record["checked"]
                .as_u64()
                .is_some_and(|t| t <= now && now - t < ttl)
            {
                let found = cached_identification(&record["match"]);
                if record["match"].is_null() || found.is_some() {
                    self.cache_hits += 1;
                    self.cache_matches += usize::from(found.is_some());
                    return found;
                }
            }
        }
        if self.offline {
            return None;
        }
        self.attempts += 1;
        let fp = cached
            .as_ref()
            .and_then(|v| serde_json::to_vec(&v["fp"]).ok())
            .and_then(|v| parse_fingerprint(&v))
            .or_else(|| self.backend.fingerprint(path))?;
        let mut record =
            serde_json::json!({"fp": {"duration": fp.duration, "fingerprint": fp.fingerprint}});
        // Keep fingerprint generation even when the network fails; never cache the failure.
        if let Some((cache, key)) = self.cache.as_ref().zip(cache_key.as_ref())
            && source_stamp.is_some()
            && crate::source::SourceStamp::at(path).ok() == source_stamp
        {
            let _ = cache.write(key, &record);
        }
        if let Some(last) = self.last_start {
            let remaining = INTERVAL.saturating_sub(self.backend.now().saturating_sub(last));
            if !remaining.is_zero() {
                self.backend.sleep(remaining);
            }
        }
        self.last_start = Some(self.backend.now());
        let response = self.backend.request(&self.key, &fp)?;
        let json: Option<Value> = serde_json::from_slice(&response.body).ok();
        let rejected = matches!(response.status, 401 | 403)
            || json.as_ref().is_some_and(|j| {
                j["status"] == "error" && matches!(j["error"]["code"].as_u64(), Some(4 | 17))
            });
        if rejected {
            self.disabled = true;
            self.warning_pending = true;
            return None;
        }
        if response.status != 200 {
            return None;
        }
        let json = json?;
        if json["status"] != "ok" || !json["results"].is_array() {
            return None;
        }
        let found = select(&json);
        // Never persist a server echo of the credential.
        if found
            .as_ref()
            .is_some_and(|f| f.artist.contains(&self.key.0) || f.title.contains(&self.key.0))
        {
            return None;
        }
        record["checked"] = now.into();
        record["match"] = found.as_ref().map_or(
            Value::Null,
            |f| serde_json::json!({"artist": f.artist, "title": f.title}),
        );
        if let Some((cache, key)) = self.cache.as_ref().zip(cache_key.as_ref())
            && source_stamp.is_some()
            && crate::source::SourceStamp::at(path).ok() == source_stamp
        {
            let _ = cache.write(key, &record);
        }
        self.successes += usize::from(found.is_some());
        found
    }
}
fn cached_identification(value: &Value) -> Option<Identification> {
    let artist = value["artist"].as_str()?.trim();
    let title = value["title"].as_str()?.trim();
    if artist.is_empty()
        || title.is_empty()
        || artist.chars().chain(title.chars()).any(char::is_control)
    {
        return None;
    }
    Some(Identification {
        artist: artist.into(),
        title: title.into(),
    })
}
fn select(json: &Value) -> Option<Identification> {
    if json["status"] != "ok" {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut groups: BTreeMap<(String, String), (usize, f64, Identification)> = BTreeMap::new();
    let mut total = 0;
    for result in json["results"].as_array()? {
        let Some(score) = result["score"].as_f64().filter(|s| (0.0..=1.0).contains(s)) else {
            continue;
        };
        let Some(recordings) = result["recordings"].as_array() else {
            continue;
        };
        for recording in recordings {
            let Some(title) = recording["title"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let Some(artists) = recording["artists"].as_array().filter(|a| !a.is_empty()) else {
                continue;
            };
            let names: Option<Vec<_>> = artists
                .iter()
                .map(|a| a["name"].as_str().map(str::trim).filter(|s| !s.is_empty()))
                .collect();
            let Some(names) = names else { continue };
            let artist = names.join(" & ");
            let pair = (artist.to_lowercase(), title.to_lowercase());
            // A repeated MusicBrainz recording ID does not provide independent support.
            if let Some(id) = recording["id"].as_str().filter(|id| !id.is_empty())
                && !seen.insert(id.to_owned())
            {
                continue;
            }
            total += 1;
            let entry = groups.entry(pair).or_insert((
                0,
                score,
                Identification {
                    artist,
                    title: title.into(),
                },
            ));
            entry.0 += 1;
            entry.1 = entry.1.max(score);
        }
    }
    let mut ranked: Vec<_> = groups.into_values().collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.0));
    let winner = ranked.first()?;
    let runner = ranked.get(1).map_or(0, |x| x.0);
    if (winner.0 >= 2 && winner.0 > runner) || (total == 1 && winner.1 >= 0.95) {
        Some(ranked.remove(0).2)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;
    struct Fake {
        replies: VecDeque<Option<Response>>,
        now: Duration,
        starts: Vec<Duration>,
        fingerprints: usize,
        fp_ok: bool,
    }
    impl Backend for Fake {
        fn fingerprint(&mut self, _: &Path) -> Option<Fingerprint> {
            self.fingerprints += 1;
            self.fp_ok.then(|| Fingerprint {
                duration: 120,
                fingerprint: "ABC".into(),
            })
        }
        fn request(&mut self, _: &ApiKey, _: &Fingerprint) -> Option<Response> {
            self.starts.push(self.now);
            self.replies.pop_front().flatten()
        }
        fn now(&self) -> Duration {
            self.now
        }
        fn sleep(&mut self, d: Duration) {
            self.now += d;
        }
    }
    fn session(replies: Vec<Option<Response>>) -> Session<Fake> {
        Session::with_backend(
            ApiKey::new("test-secret-key".into()),
            Fake {
                replies: replies.into(),
                now: Duration::ZERO,
                starts: vec![],
                fingerprints: 0,
                fp_ok: true,
            },
        )
    }
    fn response(status: u16, value: Value) -> Option<Response> {
        Some(Response {
            status,
            body: serde_json::to_vec(&value).unwrap(),
        })
    }
    fn recording(id: &str, artist: &str, title: &str) -> Value {
        json!({"id":id,"title":title,"artists":[{"name":artist}]})
    }
    fn result(score: f64, recordings: Vec<Value>) -> Value {
        json!({"status":"ok","results":[{"score":score,"recordings":recordings}]})
    }
    #[test]
    fn persistent_cache_reuses_matches_and_refreshes_expired_results() {
        let root = std::env::temp_dir().join(format!(
            "alb-cache-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("song");
        std::fs::write(&source, b"audio bytes").unwrap();
        let cache_dir = root.join("cache");
        let mut first = session(vec![response(
            200,
            result(0.99, vec![recording("1", "Artist", "Title")]),
        )]);
        first.cache = Some(crate::fingerprint_cache::Cache::new(cache_dir.clone()).unwrap());
        assert!(first.identify(&source).is_some());
        let mut second = session(vec![]);
        second.offline = true;
        second.cache = Some(crate::fingerprint_cache::Cache::new(cache_dir.clone()).unwrap());
        assert!(second.identify(&source).is_some());
        assert_eq!(
            (
                second.attempts,
                second.cache_hits,
                second.backend.fingerprints
            ),
            (0, 1, 0)
        );
        let key = crate::fingerprint_cache::Cache::key(&source).unwrap();
        let cache = second.cache.as_ref().unwrap();
        let mut record = cache.read(&key).unwrap();
        record["checked"] = 1.into();
        cache.write(&key, &record).unwrap();
        assert!(second.identify(&source).is_none());
        assert_eq!((second.attempts, second.backend.fingerprints), (0, 0));
        let mut third = session(vec![response(200, result(0.0, vec![]))]);
        third.cache = Some(crate::fingerprint_cache::Cache::new(cache_dir.clone()).unwrap());
        assert!(third.identify(&source).is_none());
        assert_eq!((third.attempts, third.backend.fingerprints), (1, 0));
        assert!(third.identify(&source).is_none());
        assert_eq!(third.cache_hits, 1);
        std::fs::write(&source, b"changed bytes").unwrap();
        assert!(third.identify(&source).is_none());
        assert_eq!(third.backend.fingerprints, 1);
        // Network failure retained a fingerprint, but not a negative lookup result.
        assert!(third.identify(&source).is_none());
        assert_eq!(third.attempts, 3);
        assert_eq!(third.backend.fingerprints, 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_formats_zero_and_fractional_success_rates() {
        let mut lookup = AcoustId::new(ApiKey::new("local-test-key".into()));
        assert_eq!(
            lookup.summary(),
            "Fingerprinting: new: 0 attempts; cached: 0 matched, 0 unmatched."
        );
        lookup.session.attempts = 3;
        lookup.session.successes = 1;
        assert_eq!(
            lookup.summary(),
            "Fingerprinting: new: 3 attempts, 1 matched (33.3%); cached: 0 matched, 0 unmatched."
        );
    }

    #[test]
    fn summary_distinguishes_cached_matches_from_unmatched_results() {
        let mut lookup = AcoustId::new(ApiKey::new("local-test-key".into()));
        for (attempts, matched, cached, cached_matches, expected) in [
            (
                0,
                0,
                22,
                0,
                "Fingerprinting: new: 0 attempts; cached: 0 matched, 22 unmatched.",
            ),
            (
                10,
                4,
                879,
                424,
                "Fingerprinting: new: 10 attempts, 4 matched (40.0%); cached: 424 matched, 455 unmatched.",
            ),
            (
                2,
                2,
                3,
                3,
                "Fingerprinting: new: 2 attempts, 2 matched (100.0%); cached: 3 matched, 0 unmatched.",
            ),
            (
                11,
                0,
                0,
                0,
                "Fingerprinting: new: 11 attempts, 0 matched (0.0%); cached: 0 matched, 0 unmatched.",
            ),
        ] {
            lookup.session.attempts = attempts;
            lookup.session.successes = matched;
            lookup.session.cache_hits = cached;
            lookup.session.cache_matches = cached_matches;
            assert_eq!(lookup.summary(), expected);
        }
    }

    #[test]
    fn statistics_count_attempts_matches_and_disabled_skips() {
        let mut s = session(vec![
            response(200, result(0.99, vec![recording("1", "Artist", "Title")])),
            None,
        ]);
        assert!(s.identify(Path::new("one.mp3")).is_some());
        assert!(s.identify(Path::new("two.mp3")).is_none());
        s.backend.fp_ok = false;
        assert!(s.identify(Path::new("three.mp3")).is_none());
        s.disabled = true;
        assert!(s.identify(Path::new("four.mp3")).is_none());
        assert_eq!((s.attempts, s.successes), (3, 1));
    }

    #[test]
    fn consensus_not_order_selects_dominant_pair_and_rejects_ties() {
        let a = recording("1", "Artist", "Title");
        let b = recording("2", " artist ", " TITLE ");
        let other = recording("3", "Other", "Song");
        let found = select(&result(0.8, vec![other.clone(), a.clone(), b.clone()])).unwrap();
        assert_eq!(found.artist.to_lowercase(), "artist");
        assert_eq!(found.title.to_lowercase(), "title");
        assert!(select(&result(1.0, vec![a.clone(), other.clone()])).is_none());
        assert!(
            select(&result(
                1.0,
                vec![a, b, other, recording("4", "Other", "Song")]
            ))
            .is_none()
        );
    }
    #[test]
    fn single_requires_high_result_score_and_duplicate_ids_do_not_add_support() {
        let a = recording("1", "Artist", "Title");
        assert!(select(&result(0.95, vec![a.clone()])).is_some());
        assert!(select(&result(0.949, vec![a.clone()])).is_none());
        assert!(select(&result(0.8, vec![a.clone(), a])).is_none());
        assert!(select(&result(1.0, vec![recording("1", " ", "Title")])).is_none());
    }
    #[test]
    fn fingerprint_validation_and_missing_or_failed_tool_skip_http() {
        assert!(parse_fingerprint(br#"{"duration":120.5,"fingerprint":"AQAB_abc-123"}"#).is_some());
        for data in [
            b"invalid".as_slice(),
            br#"{"duration":0,"fingerprint":"ABC"}"#,
            br#"{"duration":120,"fingerprint":""}"#,
            br#"{"duration":120,"fingerprint":123}"#,
        ] {
            assert!(parse_fingerprint(data).is_none());
        }
        let mut s = session(vec![]);
        s.backend.fp_ok = false;
        assert!(s.identify(Path::new("missing.flac")).is_none());
        assert!(s.backend.starts.is_empty());
        assert!(!s.disabled);
    }
    #[test]
    fn subprocess_missing_failure_and_unusable_output_are_nonfatal() {
        let executable = std::env::current_exe().unwrap();
        // A child of an executable file cannot exist as an executable path.
        assert!(run_fingerprint(&mut Command::new(executable.join("missing-fpcalc"))).is_none());
        assert!(
            run_fingerprint(Command::new(&executable).arg("--not-a-test-harness-option")).is_none()
        );
        assert!(run_fingerprint(Command::new(&executable).arg("--help")).is_none());
    }
    #[test]
    fn explicit_key_rejection_disables_subsequent_files_and_warns_once() {
        for reply in [
            response(
                200,
                json!({"status":"error","error":{"code":4,"message":"test-secret-key"}}),
            ),
            response(400, json!({"status":"error","error":{"code":17}})),
            response(401, json!({})),
            response(403, json!({})),
        ] {
            let mut s = session(vec![reply]);
            assert!(s.identify(Path::new("one.flac")).is_none());
            assert_eq!(s.take_warning(), Some(REJECTED_WARNING));
            for _ in 0..3 {
                assert!(s.identify(Path::new("two.mp3")).is_none());
            }
            assert!(s.take_warning().is_none());
            assert_eq!(s.backend.starts.len(), 1);
            assert_eq!(s.backend.fingerprints, 1);
            assert!(!REJECTED_WARNING.contains("test-secret-key"));
        }
    }
    #[test]
    fn transient_errors_and_bad_responses_allow_later_requests_with_fixed_spacing() {
        let good = result(1.0, vec![recording("1", "Artist", "Title")]);
        let mut s = session(vec![
            None,
            response(503, json!({"status":"error","error":{"code":13}})),
            Some(Response {
                status: 200,
                body: b"invalid json test-secret-key".to_vec(),
            }),
            response(200, json!({"status":"error","error":{"code":14}})),
            response(200, good),
        ]);
        for _ in 0..4 {
            assert!(s.identify(Path::new("a.wav")).is_none());
            assert!(!s.disabled);
        }
        assert!(s.identify(Path::new("a.wav")).is_some());
        assert!(s.take_warning().is_none());
        assert_eq!(
            s.backend.starts,
            (0..5).map(|n| INTERVAL * n).collect::<Vec<_>>()
        );
    }
    #[test]
    fn secrets_are_redacted_and_cannot_be_echoed_into_recovered_metadata() {
        assert_eq!(
            format!("{:?}", ApiKey::new("test-secret-key".into())),
            "[REDACTED]"
        );
        let mut s = session(vec![response(
            200,
            result(1.0, vec![recording("1", "test-secret-key", "Title")]),
        )]);
        assert!(s.identify(Path::new("a.wav")).is_none());
    }
}
