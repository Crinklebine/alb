# ALB — Audio Library Builder

**Turn one or more music folders into a single organized audio library.** ALB
copies your files into a separate output folder, sorts music using its tags,
and leaves your source files unchanged. It runs on **macOS, Linux, and Windows**.

- **Combine libraries:** collect folders from one or several drives into one output.
- **Organize five audio formats:** FLAC, M4A, MP3, OGG, and WAV, with Artist/Album
  folders and Loose Tracks for music without an album.
- **Avoid exact duplicates:** compare file contents across all inputs and retain
  one verified representative within each supported format.
- **Recover missing identification:** optionally use AcoustID to find Artist and
  Title, write recovered fields to new copies, and place them under Fingerprinted.
- **Validate the final library:** normalize repairable metadata and require strict
  read-back, preserving encoded audio and artwork.
- **Handle imperfect collections:** detect mislabeled formats and obvious damage,
  report conflicting tags, and preserve unresolved files in Problem Files.
- **Preview and resume:** inspect a library, preview a build without writing library files,
  or safely resume an interrupted run.
- **Preserve archival information:** retain modification times, preserve creation
  times where the platform supports it, and record original timestamps in reports.
- **Show useful progress:** display operation counts, check destination free space,
  and keep detailed per-file results in `_ALB` reports.

This README describes **ALB 0.4.13**.

## Install

With Rust and Cargo installed, install the published release on any supported
platform:

```sh
cargo install alb --locked
```

For the development version, run this from the repository folder:

```sh
cargo install --path . --locked
```

These Cargo commands work in macOS/Linux terminals and Windows PowerShell.
Check the installed version with `alb --version`. The executable is `alb` on
macOS/Linux and `alb.exe` on Windows. Source builds require Rust 1.89 or later.

Distribution is through [crates.io](https://crates.io/crates/alb), not GitHub
Releases. AcoustID lookup additionally requires `fpcalc`; see its section below.

## Quick start

Choose an output folder separate from every input. Start with `--dry-run`, review
the summary, then run the same build without `--dry-run` to create the library.
Quote paths containing spaces.

### macOS — Terminal (zsh or bash)

```sh
# Inspect a folder.
alb scan --input "$HOME/Music/Incoming"

# Preview, then build on an external drive.
alb build --input "$HOME/Music/Incoming" --output "/Volumes/Music Drive/Library" --dry-run
alb build --input "$HOME/Music/Incoming" --output "/Volumes/Music Drive/Library"

# Combine two folders into a separate library.
alb build --input "$HOME/Music/Incoming" --input "/Volumes/Archive/Music" --output "/Volumes/Music Drive/Combined"

# Resume that combined build.
alb build --input "$HOME/Music/Incoming" --input "/Volumes/Archive/Music" --output "/Volumes/Music Drive/Combined" --resume
```

### Linux — terminal (bash or similar)

```sh
# Inspect a folder.
alb scan --input "/mnt/music/Incoming"

# Preview, then build on the mounted drive.
alb build --input "/mnt/music/Incoming" --output "/mnt/music/Organized" --dry-run
alb build --input "/mnt/music/Incoming" --output "/mnt/music/Organized"

# Combine folders from different drives.
alb build --input "/mnt/music/Incoming" --input "/mnt/archive/Music" --output "/mnt/music/Combined"

# Resume that combined build.
alb build --input "/mnt/music/Incoming" --input "/mnt/archive/Music" --output "/mnt/music/Combined" --resume
```

### Windows — PowerShell

```powershell
# Inspect a folder.
alb.exe scan --input "D:\Music\Incoming"

# Preview, then build on another drive.
alb.exe build --input "D:\Music\Incoming" --output "E:\Audio Library" --dry-run
alb.exe build --input "D:\Music\Incoming" --output "E:\Audio Library"

# Combine folders from different drives.
alb.exe build --input "D:\Music\Incoming" --input "F:\Archive\Music" --output "E:\Combined Library"

# Resume that combined build.
alb.exe build --input "D:\Music\Incoming" --input "F:\Archive\Music" --output "E:\Combined Library" --resume
```

The paths above are examples; substitute your own folders. The same options work
on all three platforms. Existing output files are never overwritten.

## Command reference

```text
alb --help
alb --version
alb --clear-cache
alb scan --input SOURCE [--input SOURCE ...] [--verbose] [--hash] [--acoustid-key KEY]
alb build --input SOURCE [--input SOURCE ...] --output DESTINATION [--dry-run] [--resume] [--acoustid-key KEY]
```

| Option | Commands | Meaning |
| --- | --- | --- |
| `--input SOURCE` | `scan`, `build` | Required source folder; repeat for multiple inputs. |
| `--output DESTINATION` | `build` | Required output folder, separate from all inputs. |
| `--dry-run` | `build` | Plan and check space without creating output files or reports. |
| `--resume` | `build` | Verify and reuse matching existing outputs. |
| `--verbose` | `scan` | Print per-file classification, metadata, and errors. |
| `--hash` | `scan` | Hash files and report same-format exact duplicate groups. |
| `--no-fingerprint-cache` | `scan`, `build` | Disable fingerprint cache reads and writes. |
| `--clear-cache` | Top level | Clear the fingerprint cache and exit. |
| `--acoustid-key KEY` | `scan`, `build` | Optional online Artist/Title lookup for missing fields. |
| `-h`, `--help` | Top level, `scan`, `build` | Show help for that command. |
| `-V`, `--version` | Top level | Show the installed version. |

Use separate option values, such as `--input "Music"`, not `--input=Music`.
Prefix relative paths beginning with `-` with `./`. Only `--input` is repeatable.
`--dry-run --resume` previews a resumed build without writing library files or
reports; optional lookup may update the fingerprint cache.

### Scan, dry-run, build, and resume

- **Scan:** source-preserving inventory, classification, and metadata inspection. Add
  `--verbose` for individual files or `--hash` for exact duplicates.
- **Dry-run:** reads metadata, hashes files, plans destinations, and checks space.
  Shows summary counts; creates no library files, directories, or reports.
  Optional fingerprint lookup can update the ALB configuration cache.
- **Build:** copies files, verifies them, and records the outcome. File-level
  problems are handled separately so unrelated work can continue.
- **Resume:** regenerates the plan and verifies existing outputs before reuse.
  Old partials and mismatched outputs stay untouched. Incomplete copies restart
  under new exclusive partial names; audit files are not used as trusted state.
  Failed working copies created during the current run are removed after a verified
  Problem Files fallback succeeds. If the fallback fails, the partials are retained.

With an AcoustID key, both `scan` and `--dry-run` can make online lookups, although
neither writes recovered tags to your files. Build terminal output contains
progress and summaries; full per-file plans and results go in `_ALB/build-*.txt`.

Exit status is **0** when requested work is handled, **1** for unhandled
operational/safety failures or scan inspection errors, and **2** for usage errors.
Successfully quarantined problem files do not cause build failure. A tolerant
metadata read can still produce a scan warning/error status while supplying
usable tags for a build.

## Combining multiple libraries

Repeat `--input` to combine several folders into one output:

```sh
alb build --input "/path/library-one" --input "/path/library-two" --output "/path/combined" --dry-run
alb build --input "/path/library-one" --input "/path/library-two" --output "/path/combined"
```

The folders can be on different drives. ALB makes one combined plan, identifies
exact duplicates across all inputs, resolves destination filename conflicts, and
checks the total required space on the output drive. Each source stays unchanged.
Use `--resume` with the same input folders and output to continue a run.

`scan` also accepts repeated inputs, including with `--hash`. Every input must
exist. Duplicate, aliased, or nested input roots are rejected to avoid processing
the same tree twice. The output must be separate from every input.

For unknown files, fingerprinted tracks, and problem files, preserved folders are
relative to each file's own input root. Matching relative folder names can merge;
conflicting filenames are disambiguated without overwriting files. The `_ALB`
report records all input roots and the original absolute path of every file.
Single-input commands continue to work as before.

## Organization

ALB identifies supported audio containers from file contents. Filename extensions
are hints, not the final decision. For example, WAV data named `.mp3` is organized
under `WAV` with a `.wav` output extension. Recognized audio with an unfamiliar
extension is handled as audio; ordinary non-audio files remain `UNKNOWN`.

| Detected format | Output group | Output extension |
| --- | --- | --- |
| FLAC | FLAC | `.flac` |
| MP4/M4A audio | M4A | `.m4a` |
| MPEG audio | MP3 | `.mp3` |
| Supported Ogg audio | OGG | `.ogg` |
| RIFF/WAV | WAV | `.wav` |
| Other files | UNKNOWN | Source extension retained |

Corrections are recorded in the report. Unrecognized or unsupported containers
with a supported audio extension are retained as problem files for review.

```text
TYPE/Album Artist or Artist/Album/[DISC-]TRACK - TITLE - ARTIST.extension
TYPE/Artist/Loose Tracks/Title - Artist.extension
UNKNOWN/source-relative-path
Problem Files/Problem Type/source-relative-folders/source filename.extension
_ALB/build-*.txt
```

Missing/blank albums with usable artist/title use Loose Tracks. Other metadata
problems are retained in Problem Files, preserving source folders and filenames.
Problem filenames get a short suffix only when names collide. Names are sanitized
conservatively and shortened when required by portable path limits.
Normalized folder aliases use deterministic spelling; different files targeting
one filename receive distinct source-path-based suffixes. No arbitrary winner
is selected. Names assume unchanged input roots and conflict membership.

Dedupe uses whole-file BLAKE3 within each detected audio type, never decoded audio.
A duplicate is omitted only after a verified representative exists. UNKNOWN
files and problem files retain independent copies. Cross-type files remain separate.
No audio-validity, audio-equivalence or quality claim is made.

## Metadata validation and normalization

New audio copies enter the organized library only with nonblank Artist and Title,
valid sorting text, and a successful **strict metadata read-back**. Album and
Album Artist are preserved when available; missing optional fields are acceptable.
This is a defined technical validation gate, not a guarantee that artist names,
song titles, or other tags are factually correct.

ALB reads imperfect source metadata using strict, best-attempt, then relaxed
parsing. It trims surrounding whitespace and normalizes Unicode in sorting
fields. Embedded control characters that cannot be safely interpreted require
review. Missing track numbers omit filename numbering rather than blocking a file.

For MP3 files with consecutive leading ID3v2 blocks, the first nonblank value wins
for each field. Later blocks and ID3v1 fill gaps; conflicting Artist, Title, and
Album values remain in the report. When repair is needed, ALB consolidates leading
ID3v2 blocks into one tag, rewrites recoverable metadata through the tag library,
and synchronizes identity in existing legacy tags. Malformed fields that relaxed
parsing discards may be omitted during repair; the source and parser warning remain
available. Processing is bounded to 32 ID3 blocks, 16 MiB per block, and 32 MiB of
leading ID3 data. Unsupported repairs are quarantined rather than accepted.

Already compliant copies need no tag rewrite. Repairs happen only on newly copied
output files. After repair, required fields must read back, the encoded audio
payload must match, and existing readable artwork must remain. Valid unrelated
tags are retained through the format's tag writer; conflicting artwork that cannot
be merged safely requires review. Failed repairs go to
`Problem Files/Metadata Write Errors` as unchanged source copies, with the failing
stage and underlying error in the explanation. Existing outputs are never repaired
in place or overwritten.

### Early damage detection

Before metadata lookup, ALB rejects empty audio files, files containing only zero
bytes, and structurally truncated supported containers. Detected damage goes to
`Problem Files/Damaged Files`; these files do not invoke `fpcalc` or AcoustID.
An understated WAV RIFF length can be repaired if its actual chunks remain intact.
WAV files with external ID3 tags are recognized and their tags moved inside the
output WAV container, while preserving encoded audio. Invalid ID3 comment language
bytes are repaired to `und` (undetermined), preserving comment text and descriptions.
Repairs that would create duplicate comment identities remain for manual review. A playable file can still
be truncated: players may skip a damaged final packet or stop early.

These checks validate container boundaries and tag readability, not every decoded
audio sample. Some damaged streams may still require an audio decoder to diagnose.
Chained/multiplexed Ogg and oversized metadata are conservatively retained for
manual review. Damaged originals are preserved; missing audio cannot be invented.

Missing or malformed ID3v1 years are normalized to `0000` (unknown); valid years
are retained. ALB does not invent a recording date.

When Unicode identity text cannot fit the ID3v1 character set, ALB preserves
legacy fields in ID3v2 and removes the incompatible ID3v1 tag from the output.
Empty Album Artist values are normalized to absent. Provably empty, invalid
ID3 identifier and truncated comment frames are removed; useful comments,
artwork, and encoded audio remain protected by the normal checks. Recognized
legacy `MP3ext ` padding is normalized to zero padding; genuine tag-frame
boundary errors still require review.

## Optional AcoustID metadata fallback

Recovery fills missing Artist/Title identification; it does not fact-check,
continuously retag, or canonicalize an already usable library. Accepted matches
are identification evidence, not a guarantee that the tags are factually correct.

### Persistent fingerprint cache

Fingerprint lookup automatically saves fingerprints and accepted lookup results in
ALB's configuration directory. No API keys or source paths are stored. Records are
small JSON files under `cache/fingerprints-v1/`:

| Platform | ALB configuration directory |
|---|---|
| Linux | `$XDG_CONFIG_HOME/alb/`, default `~/.config/alb/` |
| macOS | `~/Library/Application Support/alb/` |
| Windows | `%APPDATA%\alb\` |

Unchanged file contents reuse cached results across runs, renames, and output
folders. Successful matches expire after 90 days; unmatched results after 7 days.
Expired lookups reuse their saved fingerprint. Changed file contents get a new
cache entry. Network failures, invalid replies, and rejected credentials are not
cached as unmatched results. Corrupt cache records are ignored. Cache storage
failure does not prevent library processing. The versioned cache namespace allows
future fingerprint algorithm changes to invalidate incompatible entries.

The fingerprint summary separates new attempts from cached matches and cached
unmatched results, for example:

```text
Fingerprinting: new: 10 attempts, 4 matched (40.0%); cached: 424 matched, 455 unmatched.
Fingerprinting: new: 0 attempts; cached: 0 matched, 22 unmatched.
```

New attempts include unsuccessful fingerprint generation and lookup failures;
cached unmatched results mean a previous lookup found no accepted match. The
percentage applies only to new attempts and is omitted when there are none. Unexpired cached matches are used even
without a key. A key is required only for new or expired network lookups; without
one, cache misses and expired results leave the file's metadata unchanged.

Use `--no-fingerprint-cache` with `build` or `scan` to disable both cache reads and
writes. With no key, this disables fingerprint recovery entirely. With a key,
ALB performs fresh lookups without using or updating the cache:

```sh
alb build --input Audio --output Audio-Lib --no-fingerprint-cache
alb scan --input Audio --no-fingerprint-cache
```
Scan and dry-run may update this cache without modifying source audio. Keep the
ALB configuration/cache directory outside your input trees: ALB does not check
that location against input roots. Use `--no-fingerprint-cache` when necessary.

Clear the fingerprint cache without touching settings or music (all platforms):

```sh
alb --clear-cache
```

This command exits after clearing. An active build may repopulate the cache, so
clear it between runs. Existing runs from before this feature cannot populate the
cache retrospectively; the first new run builds it.


When lookup is enabled, a separate summary line reports fingerprint attempts,
accepted Artist/Title matches, and the percentage of attempts that matched. Failed
fingerprint generation counts as an attempt; skipped files do not. A successful
match does not guarantee the file will pass later metadata or audio checks.

Supply an AcoustID **application client key** with either `scan` or `build`:

```sh
alb build --input /path/to/source --output /path/to/library --acoustid-key YOUR_CLIENT_KEY --dry-run
```

Install Chromaprint's `fpcalc` executable and make it available on `PATH`.
ALB fingerprints supported audio files only when Artist or Title is missing or
blank and a key was supplied. Missing Album, Album Artist, track/disc number or
year alone never triggers lookup. Only missing Artist and Title are recovered
from the service; existing usable embedded values take precedence. Album/release
identification is not attempted, and source files are never retagged. Recovered
Artist/Title are written into the new output copy before publication. All normal
audio output also passes the metadata gate described above; readable embedded
identity, other valid metadata, and artwork are retained during repair. No audio
transcoding is performed. If tagging fails, the original bytes are copied to
`Problem Files/Metadata Write Errors` with an explanation.

Tracks with successfully recovered metadata are placed under
`TYPE/Artist/Fingerprinted/source-relative-folders/`, for example
`FLAC/Artist/Fingerprinted/Original Album/song.flac`. Artist uses the track artist,
preserving an embedded value when available. All folders below the input root are
retained (with portable name sanitization), rather than replaced by inferred album
organization. All problem files go to the main `Problem Files/<Problem Type>/`
area, including identified tracks, preserving source-relative folders where safe.
Their explanations retain AcoustID provenance. Failed or ambiguous lookups
do not mark a track as fingerprinted. The audit records the recovery. Exact
duplicates still share one verified copy, marked if any member was identified.

Lookups send the fingerprint and duration to the
[AcoustID lookup service](https://acoustid.org/webservice), including during a
dry-run. Requests are sequential, at least 350 ms apart (below three per second).
The service's usage terms apply. No audio file is uploaded.

Candidate selection groups recording artists/titles by trimmed, case-insensitive
text. A winning pair needs at least two supporting recordings and more support
than any runner-up. A single usable recording requires a fingerprint match score
of at least 0.95. Repeated recording IDs do not add independent support.

Missing/failed `fpcalc`, invalid responses, ambiguous matches and network failures
leave normal missing-metadata handling in place. Explicit key rejection disables
lookups for the rest of that run and produces one warning; transient failures do
not disable the key. Unresolved problems go to the central Problem Files directory
with an explanation; source safety errors are never ignored.

ALB never prints or persists the key in its reports or errors. As with other
command-line secrets, your shell history or operating system's process listing
may expose the argument; keep your key private.

## Problem Files and reports

File-level errors do not stop unrelated work. Classes include missing metadata,
metadata parsing/writing, damaged files, long paths, read errors, destination conflicts and copy errors.
Problem reports put the reason and missing fields first, followed by clearly labelled
processing results, file locations, additional details, original timestamps, and next steps.

Each handled problem copy has an adjacent text explanation with the source path,
original destination, detailed causes and outcome.

All problem-file routes, including runtime copy failures, try to retain the directory structure below the input root, so
missing tags do not discard your existing artist/album folders. Folder names are
sanitized for portability. If that layout exceeds safe path limits, ALB uses a
shorter generated filename first, preserving folders such as `Clash (Live)`.
Only when the hierarchy still cannot fit does it use a flat destination and record
the reason and original path in the explanation.
Existing outputs are never moved;
resuming a library created by an older version can leave its old flat copies.

If a source cannot be read or verified, its explanation says **NOT COPIED**;
ALB continues and returns 1 after reporting failures. Unwritable problem storage
is recorded in the audit. Root isolation, output-lock and audit failures remain
fatal because safe, truthful execution is no longer possible.

Reports are synced append-only records. A final `COMPLETE` marker records successful
handling; `FINISHED_WITH_ERRORS` records remaining failures. Missing completion
indicates interruption. Existing explanations are reused only when identical;
changed explanations receive new numbered names.

### Copies with normalized or recovered tags

A normalized or retagged output can intentionally differ from its source. ALB
verifies the initial copy before tagging, strictly checks the required tags and
preserved audio, then records its final size/hash separately from source evidence.
Successful normal-library copies have a `METADATA_GATE PASSED` audit record. Original timestamps
are restored after tagging. Existing output files are never retagged or overwritten.

Resume for tagged files prepares a fresh tagged comparison copy in the output
filesystem and compares it to the existing output. A matching existing file is
reused and only that newly created comparison partial is removed. This needs
scratch space and avoids trusting a matching title alone or trusting old reports.
Unchanged copies and source-based exact deduplication retain their existing checks.

## Space, progress and safety

Capacity estimates include planned new copy bytes, 1% of those bytes, 16 KiB per
source for metadata/reports and a 16 MiB reserve. Verified resume outputs and
omitted duplicates do not add copy bytes. Checks run before output creation, again
before execution and before each new copy. This does not reserve capacity or
guarantee every quota/allocation condition.

Free space is queried on the output filesystem, using the nearest existing
ancestor when the output folder does not yet exist. It is not assumed to be the
home drive. The displayed planned size is logical file data: filesystems that
share copied blocks, such as Btrfs, can consume much less physical space. The two
build summary space checks are both before copying, not before/after readings.

Terminal status shows an updating operation/count line. Windows enables virtual
terminal output when available; redirected/unsupported consoles use bounded
stage messages instead of escape sequences.

Use stable trees under your control. ALB rejects equal/nested roots, skips source
links and special files, and never overwrites destinations. No protection is
claimed against hostile same-user mutation after verification, arbitrary mount
changes or relocation of already-open Unix directories into source. Reads may
update OS access times. Case/Unicode collision rules are conservative, not a
complete model of every filesystem. Start with a small copied sample and separate
output before processing a full library.

## Platform filesystem backends

The sorter, catalog, planner, copy verifier, reports and recovery logic are shared.
`src/platform/` implements the filesystem boundary:

| Operation | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Confined directory access | Directory handles and openat2 | Directory handles and single-component openat | Pinned ancestor handles, no delete sharing |
| Link protection | NOFOLLOW / NO_SYMLINKS | NOFOLLOW | OPEN_REPARSE_POINT; reject all reparse points |
| No-replace publication | renameat2 NOREPLACE | renameatx_np EXCL | MoveFileEx without replacement, write-through |
| Output lock | Directory flock | Directory flock | Exclusive .alb-build.lock handle |
| Space query | fstatvfs | fstatvfs | GetDiskFreeSpaceEx |
| Modification time | Restored | Restored | Restored |
| Available creation time | Archived in report | Restored and archived | Restored and archived |

Windows retains an empty `.alb-build.lock` in output; it is not a stale lock after
the process exits. Windows junctions, mount reparse points and cloud-placeholder
reparse files are skipped/refused rather than followed. Linux refuses subtree mount crossings during execution; macOS rejects
cross-device traversal. These checks are not protection against arbitrary mount
changes.

New copies must retain exact source timestamp precision or report a problem.
Dates in audits and problem explanations use
`YYYY-MM-DD HH:MM:SS.nnnnnnnnn UTC`. Unsupported source creation times are marked
unavailable, never substituted with ctime. Keep `_ALB` reports as part of the archive.
Each duplicate source retains its own timestamps in the audit; the shared output
uses the representative's times. Existing files are never retimestamped, including
files that might be hardlinked to source.

Linux/macOS sync parent directories as well as files. Windows flushes new files
and requests write-through publication; it has no general directory-flush API.
Crash durability therefore depends on the filesystem. Supported operations are
required; ALB does not fall back to overwriting or following links.

## Build and test

Rust 1.89 or later; edition 2024. Released registry dependencies only.

```sh
cargo build --release --locked
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo install --path . --locked
```

Executable: `target/release/alb` on Linux/macOS, `target/release/alb.exe` on Windows.
The manually started GitHub Actions matrix tests all three platforms and verifies
release builds. Compressed, stripped executables are temporary CI artifacts,
retained for one day; they are not distribution releases. Install from crates.io
for normal use. Build locally for other CPU architectures.
Synthetic audio fixtures require no encoder during tests.

Maintainer documentation: [PROJECT.md](PROJECT.md) defines scope and contributor
workflow; [DECISIONS.md](DECISIONS.md) explains architectural rationale;
[STATUS.md](STATUS.md) records validation status; [NEXT.md](NEXT.md) lists near-term
validation work. This README is the primary user reference.

## Planned features

A separate, opt-in AcoustID contribution workflow is under consideration. There
is currently **no `alb share` command**. Builds and lookups do not submit tags or
fingerprints as database contributions. Lookup requests only query AcoustID.

The current metadata gate and damage checks are defined above.

## License

ALB is licensed under the GNU General Public License, version 3 or (at your
option) any later version (`GPL-3.0-or-later`). See [LICENSE](LICENSE) for the
full license text.

### Metadata outcome summary

Build reports initial inspection warnings separately from final metadata outcomes:

```text
Metadata issues (files): 120 successfully resolved; 8 unresolved (2 repair attempts failed).
```

Counts are per source file, not per warning or tag. Resolved means a known metadata
issue was resolved or output tags were normalized, and the final library output
passed validation. Verified duplicate representatives and resumed outputs can
resolve known source warnings too. Unresolved includes missing required metadata
and issues without a successfully validated normal-library output. Repair failures
are a subset of unresolved files, not an additional count. Damage, filesystem
errors, and other non-metadata problems are reported separately through Problem
Files. These totals are also retained as `METADATA_SUMMARY` in the `_ALB` run report.
Scan and dry-run cannot report completed repairs because they do not process output.

### Long names, videos, and previously processed problem files

Generated audio filenames are shortened when needed to fit the conservative
240-byte path budget. Full tags remain unchanged; normal collision handling keeps
files with the same shortened name distinct. Paths whose directories leave too
little room still require review.

MP4 containers with an actual video track are preserved under `UNKNOWN`, including
files mislabeled `.m4a`. Embedded cover artwork alone does not make an audio file a
video. ALB does not extract or convert the video's audio.

When reprocessing an ALB output, a matching adjacent ALB problem report allows
leading `Problem Files/<category>` wrappers to be collapsed. Remaining source
folders and filenames are preserved. Folders without a matching report are left
intact; old reports are preserved as ordinary input files.

Missing required Artist or Title takes precedence over repairable metadata parsing
warnings in the problem category; both reasons remain in the explanation. Duplicate
problem reports explain that the representative has an unresolved problem and the
source is being preserved independently.

Zero disc numbers are treated as unspecified and removed from normalized output
tags, while valid track numbering is retained. Non-WAVE RIFF files (such as AVI
and WebP) remain `UNKNOWN`. For unknown extensions without a recognized container
or ID3 signature, MPEG detection requires two complete, consistent frames. This
avoids mistaking UTF-16 text for MP3. Extremely short or free-format untagged audio
with an unknown extension may remain `UNKNOWN`; known audio extensions retain their
existing validation path.

Problem sidecars include a plain-language reason for their category and specific
missing Artist/Title fields, including when both fields are unavailable. Folder
preservation notes are additional context, not the reason for quarantine.
