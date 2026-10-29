# Temporary upstream compatibility patches

These copies preserve the existing parent APIs while moving to published fixes for
the security advisories found by the October 28, 2026 release gate. They do not
modify cryptographic algorithms or suppress advisories. Remove each path patch
when its parent release supports the fixed dependencies and the workspace can
upgrade without losing behavior.

| Package | Source | Local changes |
| --- | --- | --- |
| aws-config 1.8.16 | crates.io, SHA-256 `50f156acdd2cf55f5aa53ee416c4ac851cf1222694506c0b1f78c85695e9ca9d` | P-256 0.14; rand 0.10; SEC1 trait names and fallible OS-seeded randomized signing. |
| tink-hybrid 0.3.0 | `warpdotdev/tink-rust` commit `54b9ac9af93b0c08b446a7bc0582836c9403a71b`, `hybrid/` | P-256 0.14; SEC1/array APIs; fallible OS-backed key generation; enable the new RNG's JavaScript backend on WebAssembly. Existing core/proto pins stay in place. |
| opentelemetry-otlp 0.32.0 | crates.io, SHA-256 `9966929966d17620d7c316c643ba62631826e10021409357772d5eea84f62c35` | Zstd requirement 0.14.3. |
| tantivy-sstable 0.7.0 | crates.io, SHA-256 `8a2cfc3ac5164cbadc28965ffb145a8f47582a60ae5897859ad8d4316596c606` | Zstd requirement 0.14.3, preserving compression and experimental features. |

Registry copies retain their normalized `Cargo.toml`, original manifest, and VCS
metadata. Cargo cache markers and per-package lockfiles are omitted; the workspace
lockfile defines the shipped graph. Licenses are preserved. OTLP's root LICENSE
comes from `ec289cb3c6f8260951699c51df968560943c1451`; Tantivy's root LICENSE and
AUTHORS come from `9e63fc508153ef770f9ff980c8fa2f11e8e2e6db`.

The lockfile resolves crypto-bigint 0.7.10 and ctutils 0.4.6, addressing
RUSTSEC-2026-0387/0385. Zstd 0.14.3, zstd-safe 8.2.0, and zstd-sys 2.1.2 address
RUSTSEC-2026-0391/0398/0403/0410. Tokio is separately updated to the patched 1.51.6
line for RUSTSEC-2026-0386.

The Corresponding Source exporter includes these tracked files and resolves their
paths only inside its temporary synthetic manifests. Archived manifests retain
relative paths, so the source archive remains portable.
