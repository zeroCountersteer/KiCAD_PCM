# TI catalog discovery

The preferred catalog source is TI's authenticated Store Inventory and Pricing API v2: `https://transact.ti.com/v2/store/products/catalog`. Authentication uses OAuth client credentials at `https://transact.ti.com/v1/oauth/accesstoken`. TI documents the full-catalog endpoint at one call per four hours, so snapshots are cached under `data/catalogs/ti/` and refresh is explicit.

Use `TI_API_CLIENT_ID` and `TI_API_CLIENT_SECRET` with `eda-fetch ti-catalog refresh`. Without credentials or a cached snapshot, ordinary `eda-fetch ti` continues using bounded WEBENCH prefix discovery. Catalog orderables are sorted by generic part then orderable number; CAD queries are deduplicated by generic product.

Sampling is available with `eda-fetch ti-catalog sample --limit N` and through
`eda-validate ti --sample --limit N`. It operates on deduplicated generic
products, retains orderable aliases in the generic-product model, and uses
`ti-stratified-v1` with the catalog response SHA for deterministic selection.
`--limit N` without `--sample` remains a catalog-order subset.

Live catalog refresh was not executed in this environment because the required credentials were not present. No live catalog count is claimed. The fixture contains two orderables mapping to one generic product and is covered by offline tests.
