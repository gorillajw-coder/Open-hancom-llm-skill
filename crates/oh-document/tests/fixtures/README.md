Synthetic fixtures only (no real documents). All goldens come from the LEGACY P0
binary built from `reference/source-snapshot/implementation/rust-core`
(debug sha256 0e49053f…), so the byte-API tests compare the new crate with the
old engine, not with itself.

- `synthetic.hwpx` = legacy `create --spec create.json`; `synthetic.*.json` =
  legacy stdout for `inspect`, `validate`, `diff A A`; `schema.json` = `schema`.
- `apply.json` / `apply.receipt.json` (+ sha256 in `bytes_api.rs`), `create.receipt.json`.
- `media.hwpx`, `media-create.json`, `cmd/*` = `scripts/gen-bytes-fixtures.py LEGACY_BIN`
  (format, paragraph, table, border, style, link, image, image-replace, image-insert).
Receipts have `path` removed: the core never returns paths.
