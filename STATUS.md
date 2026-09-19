# Current status — alb 0.1.6

Linux build, dry-run, read-only scan, exact same-type dedupe, five-format metadata,
verified copy/publication, persistent audit and verified resume are implemented.

## Latest user-trial fix

- File-level problems route to Problem Files/<Problem Type>/ with stable,
  shortened source names plus path hashes and adjacent text explanations.
- Classes include Missing Metadata, Metadata Errors, Path Too Long, Read Errors,
  Destination Conflicts, Copy Errors, Duplicate Problems and Discovery Errors.
- Explanations contain source, original destination, causes and outcome.
- Per-file execution failures do not abort unrelated work. If the source cannot
  be read/verified, an explanation says NOT COPIED; final exit status is 1.
- Problem storage failures are recorded in the audit while other files continue.
- Duplicate omission still requires a verified normal representative; problem
  duplicates are preserved independently.
- Root overlap, output lock/root access and audit failures remain fatal.
- Single-line terminal progress shows stage, processed count/total and activity
  throughout long operations. Redirected logs contain stage summaries only.
- Dry-run is read-only, including problem routing.

## Verification

108 tests pass: 70 unit and 38 integration. Formatting and all-target/all-feature
Clippy pass. Synthetic tests cover problem sidecars, source disappearance, missing
metadata, parse errors, long-path routing, output conflicts, partial retention,
continued copying, resume, no-overwrite and source immutability.
Terminal progress is also checked using a pseudo-terminal.

## Limits

Linux execution requires openat2 and supported filesystem primitives. Input/output
trees must remain under user control; hostile relocation/mount changes are outside
the supported operating model. Reads can update OS access times.
No audio decoding/validity claims, source cleanup or tag editing.
Unreachable data cannot be copied; failure reports never claim otherwise.
Resume reconstructs the plan and verifies bytes rather than parsing audit files.
Old partials remain, and changed problem explanations are retained separately.

## Archival follow-up

New copies preserve and verify exact source mtime; _ALB records original per-source
creation and modification times as readable UTC dates with nanosecond precision, including duplicate
sources. Unsupported source birth times are marked unavailable. Linux native
destination birth time cannot be restored. Problem explanations include times.
Existing outputs with a different mtime are preserved unchanged; resume routes a
fresh archival copy to Problem Files rather than mutating existing inode metadata.

Read-only space checks use available filesystem blocks on output/nearest existing
ancestor. Include bytes + 1% + 16 KiB/source + 16 MiB reserve, with checked arithmetic.
Recheck before execution and each new copy. Low initial space creates no output.
Tests cover historic fractional mtime on normal/problem copies, creation-time
audit records, source immutability, pre-1970 and leap-day timestamps and pre-write space refusal.

Build console output is now summary-only; detailed plans live in the _ALB run
report. Dry-run remains read-only with summary counts and no report creation.

Report dates now use YYYY-MM-DD HH:MM:SS.nnnnnnnnn UTC. Chrono 0.4.45 with
only its std feature provides calendar formatting; no local timezone dependency.
Existing reports are not rewritten.
