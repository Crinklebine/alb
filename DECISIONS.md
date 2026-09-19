# Architectural decisions

## 2026-09-19 — Sorter-first refocus supersedes the analyzer roadmap

The user's refocus request is the current product authority. Retire previous
codec-first grouping, audio-validation gates and decoder-development milestones.
ALB sorts and rebuilds libraries. It does not need deep audio analysis to decide
where a file belongs. Historical analyzer implementation notes have been replaced
with the decisions relevant to the sorter so a resumed session follows this scope.

## Immutable input and safe roots

Retain canonical equal/nested-root rejection, read-only discovery, symlink skips,
source identity checks and complete source-snapshot tests. Never write cache,
state, reports or audio changes inside input. Future writes belong in output,
including `_ALB/`; development notes are explicit project files outside input.
No copy executor is authorized to overwrite or move originals.

## Extension-only types

Only .flac/.m4a/.mp3/.ogg/.wav map to first-class groups, ignoring ASCII case.
Everything else is UNKNOWN, including aliases formerly treated as audio hints.
M4A and OGG internals must not determine the sorting group. UNKNOWN is a preserved
bucket for later review, not an exclusion filter. Identical content under another
supported extension is still a distinct file type.

## Generic catalog and thin adapters

Track/TrackCatalog own generic metadata, type, path, source stamp and string error
records. No sample rate, depth, channel layout, codec enum or decoder evidence is
needed by planning. The retained FLAC adapter uses Lofty for basic tags and optional
duration, with artwork disabled. Parse failures retain a generic entry; whole-file
hashing does not depend on readable tags. Other types currently use empty metadata
and source stamps until thin metadata adapters are added. No guessing of tags.

## Destinations and fallback preservation

Retain generic NFC name sanitization, illegal-character replacement, reserved-name
handling, length limits and collision reporting. Prefer album artist, then artist.
Known tagged types use TYPE/ARTIST/ALBUM/[DISC-]TRACK - TITLE - ARTIST.extension.

Without usable metadata, use TYPE/_Unsorted/source-relative-path. Unknown files
use UNKNOWN/source-relative-path. This avoids flattening unrelated folders or
inventing artist/album information. Sanitize Unicode path components; retain native
non-Unicode components. Unrepresentable/overlong paths stay in the report with an
issue. Partial metadata can retain an issue alongside its fallback proposal.
Existing output entries/aliases are snapshot conflicts, never overwrite permission.

## Exact same-type duplicate detection

Keep streaming BLAKE3 and pre/post source stamps. Group by extension-defined file
type and whole-file digest, including UNKNOWN. Different unknown extensions share
the UNKNOWN type, but no files are omitted yet. Native sorted path order selects a
deterministic reported representative, not a quality judgment. Cross-type matches
are retained. No decoded-audio hashes, fingerprinting or recording comparisons.
Plan integration is implemented below; actual copy omission remains unimplemented.

## Dependencies and removal

Remove claxon, md5, the crates.io patch override and vendor/claxon entirely. Remove
analyzer modules, CLI flag, diagnostics, validation plan evidence, analyzer tests,
silence/stereo fixtures and VALIDATION.md. No replacement decoder is needed.

Keep pinned registry lofty 0.25.2 for metadata, blake3 1.8.7 for whole-file hashes,
and unicode-normalization 0.1.25 for names. Lofty's default features remain enabled:
the previously tried no-default-features configuration failed compilation in that
release. No dependency upgrades, forks, patches or Git dependencies are introduced.

## Cleanup boundary and future execution

This session restores the pipeline and preserves working safety infrastructure;
it does not add copying opportunistically. Dry-run is advisory and never writes.
Build still returns incomplete. Connect hash evidence and omission decisions to
plans, then implement no-clobber copying, verification and resumable output state.
Source and destination race protection needs explicit execution design; current
snapshot checks must not be advertised as atomic guarantees.

## 2026-09-19 — Exact duplicate planning checkpoint

Dry-run now hashes all catalog files before planning. Retain per-source type,
BLAKE3 digest and the source stamp already checked by hashing. Attachment verifies
agreement with metadata/source identity and a fresh path snapshot. Missing, failed,
mismatched or stale evidence blocks the entry. These remain snapshot checks;
execution must recheck sources and verify the representative copy before omission.

Choose the first eligible native source path within (file type, digest). Entries
with unresolved naming issues cannot represent others. Known-type duplicates point
to the representative destination, so identical files do not create false filename
collisions. Report non-duplicate conflicts rather than invent suffixes in this
slice. If later name/output checks block a representative, block all dependent
duplicate decisions too; do not silently promote another source.

Keep every UNKNOWN file as a separate planned copy, preserving original extensions
and paths for review. Scan --hash may report matching UNKNOWN bytes, but build
plans do not omit them. This conservative exception narrows the earlier generic
same-type policy. Cross-type copies are always retained. Every source remains in
the plan, including blocked entries and advisory duplicates.

Summary duplicate counts refer to accepted DuplicateOf decisions, planned copies
to unblocked Keep decisions, collisions to affected non-duplicate/unverified files,
and metadata warnings to catalog metadata/read error records. No files are written.

Choose Loose Tracks as the future missing-album folder when artist/title are
usable; do not infer commercial singles. Its focused implementation is the next
small planning slice; source-relative fallback remains the current behavior.

## 2026-09-19 — Loose Tracks naming implemented

Missing or whitespace-only album metadata uses TYPE/Artist/Loose Tracks/Title -
Artist.extension when track artist/title are usable. Prefer track artist here even
if album artist is present, because no album grouping is known. Omit track/disc
numbers entirely for loose tracks; existing album and multi-disc naming is unchanged.
Do not infer a commercial single. Unusable artist/title keeps the source-relative
fallback and review issue. Unknown files always retain their existing layout.

Normalize absent and blank album identities for folder alias checks. A real album
literally named Loose Tracks has a distinct metadata identity: report its alias
with the generated loose folder rather than silently merging them. Ordinary
filename collisions and exact-duplicate rules remain in effect. Tests cover all
supported types plus a synthetic FLAC CLI dry run with immutable-source snapshots.

## 2026-09-19 — Verified partial staging as a bounded copy slice

Implement an internal staging primitive without enabling CLI copying. Require
separate validated roots, canonical existing parents, a source under input, and
an output name ending .alb-partial. Open source read-only; create partial with
create_new so existing files/hardlinks/symlinks are never opened for writing.
Bound transfer to the expected length plus one byte, compare source stamps, sync,
then reread the staged bytes and compare size/BLAKE3 with the plan. Reuse the
existing bounded hash reader. Do not publish, rename, delete, overwrite or resume
an existing partial. Failures remain recognizably incomplete for later recovery.

Root/parent checks are snapshots, not race protection. Do not expose this primitive
through build until directory-handle-based protections and atomic no-clobber
publication are implemented. A verified partial is not a completed copy and cannot
authorize duplicate omission. Output-directory creation, directory durability,
publication, interruption recovery and execution reports belong to later slices.
No new dependency is needed. Internal dead-code allowance is temporary pending
executor integration; tests exercise the staging API directly on synthetic trees.

## Metadata coverage required before user testing

User clarified that all five supported formats must have basic sorting-metadata
support before testing the first version on a library. Prioritize the four missing
adapters before copy completion. A copy-capable build with fallback-only handling
for these formats is not the first user-testable version. This does not defer
automated development tests or expand scope into codec/audio analysis.

## Metadata coverage milestone — all five supported types

Use existing released Lofty readers for FLAC/M4A/MP3/OGG/WAV, without adding a
decoder or codec fields. OGG delegates reader selection to Lofty solely to obtain
tags; ALB's output group remains extension-defined. Prefer primary tag fields and
fill missing values from secondary tags, important for WAV RIFF/ID3 combinations.
Absent fields stay absent, malformed files stay cataloged with warnings. Add tagged
and untagged synthetic fixtures and a full five-format dry-run immutability test.
All 82 tests and formatting/Clippy pass before starting the copy milestone.

## Linux execution, durable audit and verified resume

Use released rustix 1.1.5 for directory-handle operations, openat2 no-follow/beneath/
no-subtree-mount traversal, exclusive creation, output flock and renameat2
NOREPLACE. Sync partial data and parent directories, rehash disk-read bytes before
and after publication. The older path-based staging harness is test-only.
No source-writing handles are opened. Non-Linux execution fails explicitly.

A duplicate is omitted only after its representative is published or securely
reverified during resume; recheck duplicate source bytes and representative output.
Audit reports under _ALB are exclusively created and synced before copying; each
verified result is synced, followed by COMPLETE only on full success.

Explicit --resume reconstructs current plans and verifies existing final bytes;
it does not trust old reports. Mismatches block, old partials stay untouched, and
incomplete copies restart under new exclusive names (bounded retries). This
provides recovery without a manifest parser or destructive partial cleanup.
Reports are human-readable escaped audit records, not a portable manifest format.

Supported operating model: stable input/output trees under the user's control.
Handles prevent symlink redirection but cannot prevent a hostile same-user process
relocating held directories into source or changing mounts. Broader filesystem
collation and platform qualification remain future work. The first Linux
user-testable gate is satisfied by 95 passing synthetic automated tests.

## 0.1.1 — Preserve conflicting files instead of blocking the whole plan

User trial exposed widespread metadata folder aliases and different-byte files
with identical target names. For hash-backed plans, choose one deterministic folder
spelling and suffix every non-duplicate filename conflict using the source path
hash plus a checked counter. Keep all bytes; do not infer audio equivalence.
Duplicate destinations are updated after representative names are resolved.
Existing output conflicts and missing/failed evidence still block. Report original
and chosen destinations and competing sources. Names assume unchanged source roots
and conflict membership; resume never cleans up historical outputs.
This supersedes the earlier policy of blocking all normalized folder aliases.

## 0.1.2 — Album tracks without track numbers

Missing or zero track numbers no longer block album tracks with usable artist,
album and title tags. Preserve the album folder and use Title - Artist.extension,
without guessing numbering. Record a warning in the plan; ordinary collision
resolution still preserves conflicting files. Source tags are never changed.

## 0.1.3 — Per-file isolation and terminal status

User requires progress despite individual bad files. Route naming, metadata and
output-check problems after plan generation; retain original reasons in notes,
copy independently under a classified Problem Files folder, and publish synced
text explanations through directory handles without overwriting existing data.
Use shortened basenames plus full source-path hashes to avoid naming conflicts.

Production execution catches per-file copy/verification errors and retries in a
Copy Errors location. Missing evidence may be reread through secure handles.
If still unreadable, record NOT COPIED and continue; missing/corrupt data is never
represented as a successful copy. Return nonzero after unhandled failures, with
FINISHED_WITH_ERRORS instead of COMPLETE. Global root/lock/audit failures remain
fatal because continuing cannot satisfy source safety or truthful reporting.
Prior strict executor tests remain a test-only harness; production uses isolated
per-file execution with end-to-end continuation tests.

Progress uses a bounded-interval stderr renderer so the spinner remains active
during long individual file operations. It joins and clears before summaries;
non-terminal logs have only start/end stage messages.

## 0.1.4 — Timestamps and free space

Set and verify mtime on the verified partial before atomic publication, then sync.
Record source creation and modification times in the durable per-source audit,
including duplicates; do not pretend Linux birth time can be reset. Existing
outputs with mismatched mtime are not retimestamped (hardlink/source safety).
Record exact signed nanoseconds and explicit unavailable birth times.

Use rustix fstatvfs available blocks on the output/nearest existing directory;
estimate new copy bytes plus 1%, 16 KiB per source and 16 MiB reserve. Check before
creating output and again for each new copy. Checked arithmetic fails safely.
Capacity checks cannot reserve space or model every quota/allocation condition.

## 0.1.5 — Per-file build detail belongs in the audit

Remove the full plan from build stdout and per-entry blocker dumps from stderr.
Retain concise counts and errors, and preserve the full plan in the existing _ALB
run report. Dry-run remains strictly write-free and summary-only. Explicit scan
verbosity is unchanged. Integration checks confirm quiet console output while
SOURCE/BLAKE3/PROPOSED data remains in build reports.

## 0.1.6 — Readable UTC archive timestamps

Use YYYY-MM-DD HH:MM:SS.nnnnnnnnn UTC for creation and modified times in new
run reports and problem explanations. Retain nanosecond precision, handle dates
before 1970 correctly, and label unavailable values explicitly. Use released
Chrono 0.4.45 without default/clock features. Existing audit files stay immutable.
