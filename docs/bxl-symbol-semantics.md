# BXL symbol semantics

The parser preserves candidate `ElectricalType`, `PinType`, `Electrical`, `PinShape`, `Hidden`, `Inverted`, `Clock`, and `NoConnect` values from BXL pin records. Only unambiguous textual electrical values are mapped; unknown values remain in `source_semantics.electrical_type_raw` and map to unspecified. No pin-name heuristic is used.

The checked-in TI canonical sample has no separate explicit component-pin records, gate identifiers, or pad references. Its `pin_map` entries are therefore traceability records with `explicit: false`; equal displayed numbers are not treated as proof of a source mapping. Raw flags are retained until their BXL meanings are confirmed.
