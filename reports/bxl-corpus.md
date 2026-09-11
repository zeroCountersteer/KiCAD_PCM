# BXL corpus report

The acquired corpus currently contains 99 BXL objects. A representative 60-file size-stratified sample was inspected (20 small, 20 medium, 20 large). All inspected files were binary compressed containers and successfully decompressed with the GPL-licensed `bxl-rs` decompressor; the decoded stream was UTF-8 Ultra Librarian text.

The first end-to-end canonical conversion produced one component with 48 pins and three package patterns (`DGG48`, `DGG48-M`, `DGG48-L`) and 48 pads per pattern. The parser now extracts reusable pad stacks, package pads, symbol pins, symbol names, source units, and basic package line geometry into integer nanometres.

Known limitations:

* The decoded grammar has not yet been fully specified; fields such as electrical pin type, layer semantics, fill, arcs, polygons, and component pin-map semantics remain unknown or incomplete.
* Unit interpretation as mils is an evidence-based inference, not an explicit corpus header field.
* No confirmed multi-symbol document or embedded 3D model was observed in the inspected sample.
* The current corpus stats command is intentionally decompression-based and may take significant time for all 99 files.

The source files remain unchanged under `data/objects`; canonical JSON is written under `data/derived/components`.
