# TI full-corpus status

The package discovery portion is finalized. BXL URL acquisition remains a
separate resumable stage.

| Metric | Current value |
|---|---:|
| Package definitions | 2,906 |
| Package queries processed | 2,906 |
| Pending package queries | 0 |
| Product rows discovered | 35,574 |
| Rows with BXL | 35,574 |
| Unique BXL URLs | 8,566 |
| STEP-capable rows (metadata only) | 31,026 |
| BXL observations downloaded | 198 |
| Unique BXL byte objects | 99 |
| Unique BXL bytes downloaded | 12,008,661 |
| BXL conversion observations | 198 |
| Source packages converted | 105 |
| Unique physical geometries | 27 |
| Generated footprints | 26 |
| KiCad syntax failures | 0 |

The 131 transient package-query failures all recovered successfully. The
acquisition run was intentionally bounded and resumable; STEP remained
disabled. Currently 198 BXL observations (99 unique SHA-256 objects) are
cached out of 8,566 unique discovered BXL URLs. Full BXL acquisition can resume
without re-querying the package index.
