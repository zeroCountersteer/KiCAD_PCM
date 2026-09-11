# TI BXL catalog

The package-driven catalog uses the public TI packaging pages and requires no
Store API credentials. It queries each finite package definition with both
`packageDesignator` and `pinCount`, then keeps rows that advertise BXL. Direct
WEBENCH BXL links are retained as source provenance; STEP links are retained in
the catalog but are not downloaded by the default BXL acquisition mode.

Current partial live crawl:

| Metric | Count |
|---|---:|
| Package definitions | 2,906 |
| Package queries processed | 2,906 |
| Product rows | 35,574 |
| Unique product identities | 35,574 |
| Rows with BXL | 35,574 |
| Direct BXL URLs | 35,572 |
| Unique BXL URLs | 8,566 |
| Rows with STEP | 31,026 |
| Pending package queries | 0 |
| Transient failures remaining | 0 |

Rows are keyed by part number, package code, and pin count in the durable work
state, so one orderable part appearing in multiple package variants is not
silently collapsed. `.A`/`.B` suffixes are preserved exactly. The figures are
partial and must not be interpreted as complete TI coverage.

Recovery batches retried all 131 original transient connection failures;
all recovered successfully. The package catalog is finalized. The BXL download
queue is a separate URL-level stage and is not yet exhausted.

## Failure recovery

Completed package queries are not reprocessed after a network outage. Inspect
and recover the failed subset with:

```bash
eda-fetch ti-bxl-catalog failures
eda-fetch ti-bxl-catalog retry-failures --transient-only
```

The retry command uses bounded attempts and atomically persists state and the
partial product index after each package. Successful recovery removes that
package from the active failure set; remaining transient failures stay
retryable.
