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
- **Handle imperfect collections:** recover readable tags, report conflicting
  values, and preserve unresolved files and their folder structure in Problem Files.
- **Preview and resume:** inspect a library, preview a build without writing files,
  or safely resume an interrupted run.
- **Preserve archival information:** retain modification times, preserve creation
  times where the platform supports it, and record original timestamps in reports.
- **Show useful progress:** display operation counts, check destination free space,
  and keep detailed per-file results in `_ALB` reports.

This README describes the **0.3.0 development tree**. Features described here may
not yet be in the published crates.io release; use a local source install to test
this tree.

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
| `--acoustid-key KEY` | `scan`, `build` | Optional online Artist/Title lookup for missing fields. |
| `-h`, `--help` | Top level, `scan`, `build` | Show help for that command. |
| `-V`, `--version` | Top level | Show the installed version. |

Use separate option values, such as `--input "Music"`, not `--input=Music`.
Prefix relative paths beginning with `-` with `./`. Only `--input` is repeatable.
`--dry-run --resume` previews a resumed build without writing anything.

### Scan, dry-run, build, and resume

- **Scan:** read-only inventory, classification, and metadata inspection. Add
  `--verbose` for individual files or `--hash` for exact duplicates.
- **Dry-run:** reads metadata, hashes files, plans destinations, and checks space.
  Shows summary counts; creates no files, directories, or reports.
- **Build:** copies files, verifies them, and records the outcome. File-level
  problems are handled separately so unrelated work can continue.
- **Resume:** regenerates the plan and verifies existing outputs before reuse.
  Old partials and mismatched outputs stay untouched. Incomplete copies restart
  under new exclusive partial names; audit files are not used as trusted state.

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

Classification is solely by final extension, ignoring case:

| Extension | Output group |
| --- | --- |
| .flac | FLAC |
| .m4a | M4A |
| .mp3 | MP3 |
| .ogg | OGG |
| .wav | WAV |
| Anything else | UNKNOWN |

For example, .m4b, .mp4, .opus, .oga and .wave are UNKNOWN. M4A and OGG internal
codecs never change the group. All five known types have sorting-metadata readers.

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

Dedupe uses whole-file BLAKE3 within each known type, never decoded audio.
A duplicate is omitted only after a verified representative exists. UNKNOWN
files and problem files retain independent copies. Cross-type files remain separate.
No audio-validity, audio-equivalence or quality claim is made.

## Metadata reading and conflicts

ALB tries strict metadata parsing, then best-attempt and relaxed parsing when a
parse error occurs. Usable Artist and Title allow organization despite a strict
parser warning; the warning stays in the report. Missing track numbers do not
quarantine a file: its filename simply omits the numbering.

For MP3 files with consecutive leading ID3v2 blocks, fields are read separately.
The first nonblank value wins for each field; later blocks and then ID3v1 fill
missing fields. Blank values cannot erase usable values. Conflicting Artist,
Title, and Album values are recorded in the report; ALB does not determine which
conflicting value is factually correct. The separate-block reader is bounded to
32 blocks of at most 16 MiB each.

Metadata writes use the known format where available and read back recovered
fields using the same MP3 block precedence. AcoustID-written Artist and Title
must pass the read-back check. Existing embedded tags are not comprehensively
normalized: accepting readable tags does **not** certify that every tag is
standards-compliant or factually correct. Full metadata normalization is future
work. Files whose extensions do not match their containers can still cause
reader/writer problems; automatic format correction is not implemented.

## Optional AcoustID metadata fallback

Supply an AcoustID **application client key** with either `scan` or `build`:

```sh
alb build --input /path/to/source --output /path/to/library --acoustid-key YOUR_CLIENT_KEY --dry-run
```

Install Chromaprint's `fpcalc` executable and make it available on `PATH`.
ALB fingerprints supported audio files only when Artist or Title is missing or
blank and a key was supplied. Missing Album, Album Artist, track/disc number or
year alone never triggers lookup. Only missing Artist and Title are filled;
existing embedded values always win. Album/release identification is not attempted,
and source files are never retagged. Recovered Artist/Title are written into the
new output copy before publication. Only recovered fields are changed; embedded
values, other metadata and artwork are retained by the tag writer. No audio
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
metadata parsing, long paths, read errors, destination conflicts and copy errors.
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

### Copies with recovered tags

A retagged output is intentionally different from its source. ALB verifies the
initial copy before tagging, reopens the output to check the recovered tags, then
records its final size/hash separately from source evidence. Original timestamps
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
reparse files are skipped/refused rather than followed. macOS/Linux refuse subtree
mount crossings during execution.

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

See [PROJECT.md](PROJECT.md), [STATUS.md](STATUS.md),
[DECISIONS.md](DECISIONS.md), and [NEXT.md](NEXT.md) for project context.

## Planned features

A separate, opt-in AcoustID contribution workflow is under consideration. There
is currently **no `alb share` command**. Builds and lookups do not submit tags or
fingerprints as database contributions. Lookup requests only query AcoustID.

Comprehensive output-tag normalization and compliance validation are also planned;
they are not guarantees of the current build.

## License

ALB is licensed under the GNU General Public License, version 3 or (at your
option) any later version (`GPL-3.0-or-later`). See [LICENSE](LICENSE) for the
full license text.
