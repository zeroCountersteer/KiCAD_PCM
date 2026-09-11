# Package normalization

See the package-deduplication report for corpus output. The normalizer translates each package by its bounding-box center, tests 0/90/180/270 degree rotations, sorts structured geometry deterministically, and hashes four levels: physical, electrical, manufacturing, and full. Mirrors are not normalized away. Raw objects are never changed.
