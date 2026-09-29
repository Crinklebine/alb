# Near-term validation work

This tracks immediate fixes and validation, not a feature roadmap. Current behavior and
limitations are documented in [README.md](README.md).

1. Correct and validate tagged-output resume capacity estimates: account for
   remaining new copies and peak comparison scratch space, rather than summing
   every existing tagged output. Cover a nearly full destination where each
   sequential comparison fits but their combined size does not.
2. For changes affecting filesystem operations, run the manual native CI matrix
   and record results for the tested revision on Linux, macOS, and Windows.
   Include the 0.4.14 reprocessing changes, currently tested only on Linux.
3. Broaden filesystem and network-volume qualification with small disposable
   samples, recording timestamp precision, no-overwrite behavior, interrupted
   builds, and verified resume before attempting full-library trials.
4. Measure large-library inspection, fingerprint-cache reuse, and copy performance
   before choosing optimizations; retain source safety and verification checks.
