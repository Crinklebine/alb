# ALB — Audio Library Builder

A Rust command-line tool for rebuilding an audio library into a separate organized
output tree. Source files are never moved, deleted, retagged or intentionally
timestamped. Supports **Linux, macOS and Windows**.

## Commands

```sh
alb scan --input /path/to/source
alb scan --input /path/to/source --verbose --hash
alb build --input /path/to/source --output /path/to/library --dry-run
alb build --input /path/to/source --output /path/to/library
alb build --input /path/to/source --output /path/to/library --resume
```

Windows example (PowerShell):

```powershell
alb.exe build --input "D:\Music" --output "E:\Library" --dry-run
```

Use `alb --help`, `alb build --help`, or `alb scan --help` for all switches.
Options take separate values. Build output is progress and summaries only;
full per-file plans and results are retained in `_ALB/build-*.txt`.

- **Scan:** read-only inventory, classification and metadata. `--verbose` shows
  per-file information; `--hash` reports same-type exact duplicates.
- **Dry-run:** reads metadata, hashes files, plans destinations and checks space.
  Shows summary counts; creates no files, directories or reports.
- **Build:** verifies partial copies, publishes without replacement, verifies
  published bytes and records results. Continues after individual file problems.
- **Resume:** regenerates the plan and verifies existing outputs before reuse.
  Old partials and mismatched outputs stay untouched. Incomplete copies restart
  under new exclusive partial names; audit files are not used as trusted state.

Exit status: 0 when requested work is handled, 1 for unhandled operational/safety
failures (or scan inspection errors), 2 for usage errors. Successfully quarantined
problem files do not cause build failure.

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
Problem Files/Problem Type/short name [source-path hash].extension
_ALB/build-*.txt
```

Missing/blank albums with usable artist/title use Loose Tracks. Other metadata
problems are retained in Problem Files. Names are sanitized conservatively.
Normalized folder aliases use deterministic spelling; different files targeting
one filename receive distinct source-path-based suffixes. No arbitrary winner
is selected. Names assume unchanged input roots and conflict membership.

Dedupe uses whole-file BLAKE3 within each known type, never decoded audio.
A duplicate is omitted only after a verified representative exists. UNKNOWN
files and problem files retain independent copies. Cross-type files remain separate.
No audio-validity, audio-equivalence or quality claim is made.

## Problem Files and reports

File-level errors do not stop unrelated work. Classes include missing metadata,
metadata parsing, long paths, read errors, destination conflicts and copy errors.
Each handled problem copy has an adjacent text explanation with the source path,
original destination, detailed causes and outcome.

If a source cannot be read or verified, its explanation says **NOT COPIED**;
ALB continues and returns 1 after reporting failures. Unwritable problem storage
is recorded in the audit. Root isolation, output-lock and audit failures remain
fatal because safe, truthful execution is no longer possible.

Reports are synced append-only records. A final `COMPLETE` marker records successful
handling; `FINISHED_WITH_ERRORS` records remaining failures. Missing completion
indicates interruption. Existing explanations are reused only when identical;
changed explanations receive new numbered names.

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

## Space, progress and safety

Capacity estimates include planned new copy bytes, 1% of those bytes, 16 KiB per
source for metadata/reports and a 16 MiB reserve. Verified resume outputs and
omitted duplicates do not add copy bytes. Check before output creation, again
before execution and before each new copy. This does not reserve capacity or
guarantee every quota/allocation condition.

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
The manually started GitHub Actions matrix tests all three platforms and uploads
stripped release executables as compressed workflow artifacts, retained for three
days. Build locally for other CPU architectures.
Synthetic audio fixtures require no encoder during tests.

See PROJECT.md, STATUS.md, DECISIONS.md and NEXT.md for project context.

## License

ALB is licensed under the GNU General Public License, version 3 or (at your
option) any later version (`GPL-3.0-or-later`). See [LICENSE](LICENSE) for the
full license text.
