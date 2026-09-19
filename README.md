# ALB — Audio Library Builder

A Rust command-line sorter for rebuilding a large audio library into a separate
organized output tree. Input is always read-only.

## Current commands

```sh
cargo run -- scan --input /path/to/source
cargo run -- scan --input /path/to/source --verbose --hash
cargo run -- build --input /path/to/source --output /path/to/library --dry-run
```

Scan reports discovery and catalog counts. `--verbose` prints escaped per-file
metadata/errors; `--hash` reports exact whole-file duplicates within each file
type. Scan exits 0 on success, 1 on operational/metadata/report errors, 2 on usage
errors. Metadata errors do not remove files from the catalog.

Build execution is available on Linux. Preview first, then omit `--dry-run` to
copy into a separate output tree:

```sh
cargo run -- build --input /path/to/source --output /path/to/library
cargo run -- build --input /path/to/source --output /path/to/library --resume
```

`--dry-run` hashes every catalog file and shows summary counts without creating files or directories. Detailed source/destination
mappings, hashes and decisions are retained only in the build run report.
Build/dry-run exit 0 when all files are handled, 1 on unhandled read/write or safety failures,
and 2 on usage errors. File-level problems are preserved under Problem Files with explanations.
There is no audio-analysis command.

Build verifies partial-file size and BLAKE3, atomically publishes without replacing
an existing destination, then verifies published bytes. Exact duplicates are
omitted only after their representative is verified. Synced audit reports in
`_ALB/build-*.txt` record the plan, hash evidence and verified results. A missing
`COMPLETE` line means the run did not record successful completion.

`--resume` regenerates the plan from current sources and verifies existing outputs
before reusing them. Mismatched destinations route source files into Problem Files without overwriting them. Missing files are
copied; old partial files remain untouched and a new partial name is reserved.
Resume does not parse or trust prior audit reports. Combining `--resume --dry-run`
verifies reusable destinations without writing.

## Extension-only organization

| Final extension, ignoring case | Output group |
| --- | --- |
| .flac | FLAC/ |
| .m4a | M4A/ |
| .mp3 | MP3/ |
| .ogg | OGG/ |
| .wav | WAV/ |
| Everything else | UNKNOWN/ |

M4A/OGG internals never change the group. Unknown files are preserved and reported.
For example, .m4b, .mp4, .opus and .oga currently belong to UNKNOWN.

All five supported formats have thin metadata readers for sorting tags and optional
duration. Primary tags take precedence; secondary tags fill absent fields. Generic planning uses available metadata:

```text
TYPE/Album Artist or Artist/Album/[DISC-]TRACK - TITLE - ARTIST.extension
TYPE/Artist/Loose Tracks/Title - Artist.extension  # missing or blank album
Problem Files/Problem Type/             # problematic files plus .txt explanations
UNKNOWN/source-relative-path           # unsupported extension
_ALB/                                  # durable build audit reports
```

Loose tracks use track artist and omit track/disc numbering; this does not classify
commercial singles. Naming sanitizes components and unifies normalized folder spellings deterministically.
Conflicting filenames receive stable source-path-based suffixes, preserving every
non-identical file. The plan reports competing sources and chosen destinations. Metadata errors remain visible in the plan and problem explanations.
Exact duplicate groups use BLAKE3 over whole files, only within the same type.
Dry-run selects the first eligible source path in native sort order as the
representative, retaining hash/source-stamp evidence. Its duplicates point to the
same destination. Hash failures or changed sources are reported as problems; an unverified
representative cannot authorize omission. UNKNOWN files retain separate planned
copies even when their hashes match. Cross-type copies stay separate. No audio-equivalence or quality analysis occurs.

## Safety and limitations

Equal or nested input/output roots are rejected. Discovery skips symlinks and
special files, keeps hardlink paths, and reports per-file errors. Source stamps
check for detectable changes during metadata reads and hashing. Existing-output
checks detect occupied paths and conservative case/Unicode aliases.
Linux builds anchor operations to directory handles, reject symlink traversal and
subtree mount crossings, and lock output against another ALB build. The kernel and
filesystem must support openat2, exclusive creation, directory sync, flock and
atomic no-replace rename; unsupported operations fail without an unsafe fallback.
Scan/dry-run remain available on other platforms.

Use stable input/output trees under your control. Directory handles prevent
symlink redirection, but ALB cannot protect against a hostile same-user process
moving already-open directories into input, changing mounts, or mutating files
after verification. Case/NFC checks are conservative, not universal filesystem
collation. Reads may update OS access times. No source is renamed, moved, deleted,
retagged or intentionally timestamped.

Interrupted builds retain completed files and recognizable `.alb-partial` files.
Resume starts incomplete copies again; it does not continue at a byte offset or
delete old partials. Audit reports are not a portable machine-readable manifest.
An unchanged source snapshot is recommended for predictable resume planning.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features
```

Released dependencies: Lofty, BLAKE3, unicode-normalization and Linux-only rustix. No vendored decoder
or dependency patches. Synthetic metadata tests need no runtime audio encoder.
Read PROJECT.md, STATUS.md, DECISIONS.md and NEXT.md when resuming work.

## First controlled test

Build the executable with `cargo build --release --locked`. From the project root,
run `./target/release/alb` with the same arguments shown above.

The metadata and automated execution gates are met for a Linux trial. Use a small
copied sample of your library and a fresh, separate output folder. Review a dry-run,
build it, inspect the organized output and `_ALB` report, then try a resume dry-run.
Keep the source and output trees stable while ALB runs. Start with this limited
trial before attempting the full library.

### Version 0.1.1 — collision handling

Verified plans unify folder aliases (case, Unicode normalization and sanitization)
using the first source-sorted spelling. Different files targeting one filename
each get a deterministic source-path hash suffix; exact duplicates follow their
representative's resolved destination. Reserved names are checked again to avoid
secondary collisions. No existing output is overwritten. Unsafe paths, missing
hash evidence and names that cannot fit the path budget still block the build.

Names are stable for the same source tree; adding/removing conflicting sources
or changing input paths can change planned names. Resume never deletes older
outputs. A final summary shows resolved conflicts and up to ten remaining blockers.

## 0.1.2 — Album tracks without track numbers

Missing or zero track numbers no longer block album tracks with usable artist,
album and title tags. Preserve the album folder and use Title - Artist.extension,
without guessing numbering. Record a warning in the plan; ordinary collision
resolution still preserves conflicting files. Source tags are never changed.

## Version 0.1.3 — Problem Files and progress

A file-level problem does not stop other files. Missing metadata (including unusable
track numbers), metadata parsing errors, overlong planned paths, output conflicts
and copy failures are routed beneath `Problem Files/<Problem Type>/`.
Names combine a shortened source basename with a deterministic source-path hash.
Every successfully handled problem copy has an adjacent `.txt` explanation with
the source path, original destination, detailed causes and copy outcome.

If a file cannot be read or verified, its explanation says NOT COPIED; ALB
continues with other files and exits 1 after reporting the failures. Discovery
failures get explanations when output remains writable. If explanations cannot
be written, the _ALB audit records that failure. It cannot promise a problem copy
for unreadable/missing sources or an unwritable output.

Equal/nested roots, an unavailable output root/lock, or inability to create/update
the audit are global safety failures. Sources are never modified and output files
are never overwritten. Existing explanation files are reused only when identical;
changed explanations get new numbered names.

Interactive stderr shows one updating spinner/count line for discovery, metadata,
hashing, planning and copy/verification. Redirected stderr gets start/end stage
summaries without terminal control codes. Dry-run previews problem routing but
creates neither problem copies nor explanations.

## Version 0.1.4 — Archival timestamps and capacity checks

New normal and Problem Files copies preserve the source modification time,
including fractional seconds where supported. A filesystem that cannot retain
the exact value produces a reported copy error rather than silently losing precision.
Source files are never timestamped by ALB.

Linux does not expose a normal API to restore a copied file's filesystem birth
time. Original source creation time is therefore preserved in per-source
SOURCE_TIMES records inside the _ALB audit, alongside original modification time.
Values are readable UTC dates with nanosecond precision; unavailable source
birth times are explicitly marked unavailable, never substituted with ctime.
Keep the _ALB reports as part of the archive. Problem explanations include these
times too. A duplicate's own timestamps remain in the audit; its shared output
file uses the representative's modification time.

Resume verifies modification time as well as file bytes. An older output with
different modification time is left untouched and the source is preserved as
a new Problem Files copy; existing files, including hardlinks, are never retimestamped.

Dry-run and build estimate free space on the destination filesystem before writes.
The estimate includes planned copies, 1% of their bytes, 16 KiB per source for
metadata/reports, and a 16 MiB reserve. Verified resume outputs and omitted exact
duplicates do not add copy bytes. ALB repeats the check before execution and before
each new file copy. Insufficient initial space stops the run before output creation.
This is a conservative check, not a space reservation: quotas, concurrent writers,
filesystem allocation and unexpectedly large reports can still cause write failures.

## Version 0.1.5 — Quiet build output

Build and dry-run show progress, capacity checks and summary counts only.
Per-file SOURCE/ACTION/BLAKE3/PROPOSED/NOTE details remain in _ALB/build-*.txt for
actual builds. Dry-run creates no report or other output. Explicit scan --verbose
and scan --hash retain their requested detail.

## Version 0.1.6 — Readable archival dates

Creation and modification times in new _ALB reports and Problem Files explanations
use `YYYY-MM-DD HH:MM:SS.nnnnnnnnn UTC`, retaining fractional precision.
Example: `2000-01-01 00:00:00.123456789 UTC`. Missing dates remain unavailable.
Existing reports are retained unchanged.
