# eda-library

Raw, content-addressed acquisition of manufacturer-hosted EDA assets. This project does not scrape Ultra Librarian, convert BXL, or bypass access controls.

```bash
cargo build --release
./target/release/eda-fetch ti --limit 100 --dry-run
./target/release/eda-fetch stats

TI catalog (credentials required only for refresh):
TI_API_CLIENT_ID=... TI_API_CLIENT_SECRET=... eda-fetch ti-catalog refresh
eda-fetch ti-catalog stats
eda-fetch ti --catalog --limit 100 --dry-run

./target/release/eda-convert package-dedupe --manufacturer ti
./target/release/eda-convert package-diff DGG48 DGG48-M
./target/release/eda-convert kicad --manufacturer ti --all --deduplicated --clean
```

Architecture: `Vendor Adapter → Discovery → Asset References → Downloader → SHA256 Object Store → JSONL Manifest`.

TI uses the public WEBENCH CAD catalog (`pf_cad_data.txt`) and directly linked `.bxl`, `.stp`, and ZIP assets. Analog Devices uses the official product-category page as a bounded candidate index, then parses direct CAD links on selected product pages. Explicit URLs in `config/analog-devices-urls.txt` remain an override/debug input. See each vendor README and `FINDINGS.md`.

The downloader rejects empty responses, obvious HTML/JSON error pages, invalid ZIP signatures, and non-STEP text before moving a temporary file into the object store. Live validation is intentionally low-concurrency (`--concurrency 2`).

The transformation tool is built with `cargo build --release -p eda-convert`:

```bash
./target/release/eda-convert --data data bxl data/objects/ab/<sha256>.bxl
./target/release/eda-convert --data data bxl --mpn SN7400
./target/release/eda-convert --data data bxl --all
./target/release/eda-convert --data data bxl-stats
```

The BXL parser uses `cyrozap/bxl-rs` 0.1.0 only for decompression; see `docs/bxl-format-findings.md` for its GPL-3.0 license and the current parser boundary.

Generate modern KiCad libraries from canonical components with `eda-convert kicad`; run `eda-convert kicad-check data/generated/kicad` for structural validation. KiCad 10.0.4 was available during development and accepted the generated symbol and footprint files. SVG previews can be produced with `kicad-cli sym export svg` and `kicad-cli fp export svg`.

Objects are written as `data/objects/<first-two-hash>/<sha256>.<original-extension>`. Temporary files are used and duplicate hashes are not rewritten. Manifests preserve part, source URL, filename, checksum, size, timestamps, HTTP validators, and content type.
