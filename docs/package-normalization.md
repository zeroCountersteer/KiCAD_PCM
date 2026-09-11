# Package normalization

See the package-deduplication report for corpus output. The normalizer translates each package by its bounding-box center, tests 0/90/180/270 degree rotations, sorts structured geometry deterministically, and hashes four levels: physical, electrical, manufacturing, and full. Mirrors are not normalized away. Raw objects are never changed.
# Fingerprint policy

Physical fingerprints remain analysis-only and intentionally ignore pad
numbering and manufacturing behavior.  Shared generated KiCad footprints use
the manufacturing fingerprint, which retains numbering, drill, plating,
layers, mask, and paste fields.  Pad-only fingerprint origins are calculated
from pads, so unrelated silk/Fab geometry cannot change a pad geometry hash.
