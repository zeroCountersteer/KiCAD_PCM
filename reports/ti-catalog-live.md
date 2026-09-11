# TI catalog live validation

This report is intentionally a capability record, not a fabricated live result.

As of 2026-09-10, the validation environment did not provide
`TI_API_CLIENT_ID` or `TI_API_CLIENT_SECRET`, so an authenticated TI catalog
refresh and the 100/1000-product live runs were not executed here. No OAuth
token or secret was written to the repository or to a report.

Run the live validation locally with:

```bash
export TI_API_CLIENT_ID=...
export TI_API_CLIENT_SECRET=...
cargo build --release
./target/release/eda-validate ti --refresh-catalog --limit 100 --clean
./target/release/eda-validate ti --limit 1000 --use-cached-catalog --resume
```

The resulting immutable catalog snapshot is under `data/catalogs/ti/`, and
the consolidated run reports are under `reports/runs/ti/`. The harness records
catalog source, stage outcomes, pipeline revision, non-secret environment
metadata, and a JSONL failure ledger. Existing cached snapshots may be used
without credentials; `--refresh-catalog` requires both variables.
