# Releasing

No release is authorized from the initial checkpoint. A release requires clean validation, reviewed dependency licenses and advisories, source revision and protocol/ABI metadata, checksums, notices, compatibility records, and a matching central release manifest entry. Prerelease artifacts must never be labeled stable.

Native SDK source and build metadata remain prerelease-only. Linux packaging normalizes archive
ownership, ordering, timestamps, and gzip metadata and records per-file plus archive SHA-256 values.
Android and Apple artifact bundles include source-revision metadata covered by their checksum
manifests. Android, Windows, and Apple artifacts still require successful platform CI, symbols,
notices, and a matching compatibility record before publication. Never publish generated files from
an uncommitted or unverified worktree.
