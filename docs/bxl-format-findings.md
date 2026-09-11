# BXL corpus findings

Inspection date: 2026-09-10. The archive contained 99 BXL objects. I sampled 20 small (28,606–31,099 bytes), 20 medium (31,156–99,413 bytes), and 20 large (99,419–170,354 bytes) objects, with additional spot checks across the remaining size range.

## Observed format

The files are binary containers, not plain text. Their first four bytes encode a bit-reversed big-endian decompressed-size field; the remainder is an adaptive-Huffman bitstream. The decoded payload is UTF-8 text using a parenthesized, line-oriented Ultra Librarian grammar. A representative decoded file contains `TextStyle`, `PadStack`, `Pattern`, `Symbol`, and `Component` records.

Observed relationships:

* `PadStack` defines reusable pad geometry.
* `Pattern` defines a package and can have alternate density variants (`DGG48`, `DGG48-M`, `DGG48-L`).
* `Symbol` contains `Pin` and graphic records.
* `Component` names a component, provides `PatternName` and `AlternatePattern` references, and contains `PinMap`/`PadNum` mappings.
* At least one sampled file has one symbol, three package patterns, and 48 pins/pads. Multiple patterns per component are therefore real; symbol/package identity must not be collapsed by filename.

Coordinates in the decoded TI corpus are decimal values such as `19.685` and `300`. They are interpreted as mils by the current canonicalizer (1 mil = 25,400 nm), based on package pitch and pin lengths. This is an inference, retained in canonical metadata; the BXL grammar does not label the unit in the inspected records. Rotation values are degree-like integers and are stored as millidegrees.

The sampled objects showed no confirmed BXL version tag, no confirmed embedded STEP payload, and no multiple-symbol document. Those are unsupported/unknown rather than assumed absent globally. The parser preserves decoded records and raw text for future grammar work and skips unknown semantics safely.

## Reusable implementation

`cyrozap/bxl-rs` (crates.io `bxl` 0.1.0, repository `https://github.com/cyrozap/bxl-rs`) supplies the decompressor used by `crates/bxl/parser`. It is GPL-3.0-or-later, so the parser/converter crates are explicitly GPL-3.0-or-later and this dependency is not copied into the repository. Its API only decompresses BXL; all structural parsing and canonicalization here is project code. License compatibility should be reviewed before redistributing this workspace under another license.
