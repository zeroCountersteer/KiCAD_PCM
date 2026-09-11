# Live TI validation

Build the workspace, then run the single orchestration command:

```bash
export TI_API_CLIENT_ID=...
export TI_API_CLIENT_SECRET=...
cargo build --release
./target/release/eda-validate ti --refresh-catalog --limit 100 --clean
./target/release/eda-validate ti --limit 1000 --use-cached-catalog --resume
```

Use `--sample` for a deterministic diversity-oriented subset of generic TI
products. It groups by the richest available catalog dimension (category,
package, pin bucket, or derived prefix), selects round-robin across groups, and
uses a SHA-256/catalog-version tie-breaker. Without `--sample`, `--limit` uses
the stable catalog order. The sampler version is `ti-stratified-v1` and is
included in run IDs and reports.

For selection diagnostics without WEBENCH acquisition:

```bash
./target/release/eda-fetch ti-catalog sample --limit 100 --explain
./target/release/eda-fetch ti-catalog sample --limit 100 --json
./target/release/eda-validate ti --sample --limit 1000 --dry-run
```

Use `--dry-run` to stop after catalog-backed WEBENCH planning. `--models-only` limits acquisition to STEP/STP assets; `--no-3d` omits 3D generation; `--staged` runs 100 and then 1000 only if the first stage has no pipeline failures. Reports are under deterministic IDs in `reports/runs/ti/`, with `latest/` as a convenience pointer. Credentials and tokens are never recorded.
