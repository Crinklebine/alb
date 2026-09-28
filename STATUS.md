# Validation status

This file records validation evidence and its limits. See [README.md](README.md)
for implemented behavior and [NEXT.md](NEXT.md) for follow-up work.

## Current evidence

- User trials have exercised the same library data on Linux, macOS, and Windows.
  These trials establish practical coverage, not qualification of every filesystem.
- Automated tests cover all five formats, source preservation, malformed tags,
  problem routing, exact duplicates, resume verification, and filesystem safety.
  Platform-specific tests run only on their applicable operating system.
- The native GitHub Actions workflow is manually dispatched. Its configuration
  runs tests, formatting, Clippy, and release-build checks on all three platforms;
  configuration alone is not evidence that a particular revision passed.

## Qualification limits

There is no comprehensive qualification matrix for network volumes, filesystem
collation, timestamp precision, or crash recovery. Local test results must not be
presented as guarantees for every APFS, NTFS, or Linux filesystem configuration.
The supported operating model and durability limits are documented in README.
