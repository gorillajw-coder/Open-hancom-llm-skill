# Locally modified zune-jpeg 0.5.15

Upstream: https://crates.io/crates/zune-jpeg/0.5.15
Source repository: https://github.com/etemesi254/zune-image
Crate SHA256: `27bc9d5b815bc103f142aa054f561d9187d191692ec7c2d1e2b4737f8dbd7296`.
Copyright/attribution and LICENSE-MIT, LICENSE-APACHE, LICENSE-ZLIB are retained.
Use under MIT option. This is an altered source version, not upstream zune's
unmodified strict decoder. No endorsement or upstream fix acceptance is claimed.

Open-hancom patch identity: `open-hancom-completeness-v1`.
The upstream strict decoder admitted truncated baseline entropy via marker
zero-fill, demonstrated by real synthetic header-only/short-stream RED tests.
Changes are limited to:

* `src/bitstream.rs`: track artificial lookahead bits; reject consuming them.
* `src/mcu.rs`: strict single-interleaved-baseline requires complete pixels,
  no EOF overread/artificial bit consumption, <=7 remaining real all-one pad
  bits, actual EOI and no early successful termination/extra scan.
* `src/lib.rs`: remove obsolete unused crate-level macro_use attribute to avoid
  a future-compiler warning; no behavior change.

No entropy/Huffman/IDCT/color implementation is replaced with a bespoke stub.
The engine now gates all use to the versioned
`jfif-baseline-8bit-ycbcr-444-422-420-v2` subset before invoking this full decoder:
conventional JFIF Y=11/21/22,Cb=Cr=11, one interleaved SOF0 8-bit YCbCr scan.
The completeness-v1 source patch is UNCHANGED: subsampled MCU loops consume
all component blocks (including edge padding); RGB postprocessing handles the
vertical carry rows before the same final real-bit/complete-pixel/EOI checks.
Progressive/restart/metadata/unconventional sampling paths remain unexposed.
Subsampling source trace, checked allocation formulas and actual finite tests:
`validation/jpeg-subsampling-format/RESEARCH.md`. This is not an independent
codec audit or a native display/color-equivalence claim.

Exact upstream file hash inventory is in
`validation/jpeg-media-start/vendor-preimage-manifest.json`; exact unified patch
in `validation/jpeg-media-format/strict-completeness.patch`; public API/spec and
license/source reasoning in `validation/jpeg-media-format/RESEARCH.md`.
Project Cargo.toml's crates.io patch selects this local source; Cargo registry
source files were not edited. Final locked/offline build is required.
