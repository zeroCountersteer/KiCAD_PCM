# TI package-driven discovery

The preferred credential-free source for BXL acquisition is TI's public packaging
interface:

* `searchalltipackages.tsp` — the page embeds a JavaScript JSON list. Each item
  uses `pType`, `dgn`, `pnC`, `pit`, `hgt`, `BL`, and `BW` for family, designator,
  pin count, pitch, height, length, and width.
* `searchproductbypackage.tsp` — a normal server-rendered result page. The form
  submits `packageDesignator`, `pinCount`, and `results=results`.

The product rows preserve the returned part number, including `.A` and `.B`
variants, and retain product/package metadata. CAD columns link directly to
WEBENCH `dlbxl.cgi` resources such as
`https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/<part>_<package>_<pins>.bxl` and
`https://webench.ti.com/cad/dlbxl.cgi/newstep/<package><pins>A.stp`.
Direct BXL links are used by acquisition, avoiding a second `cad.cgi` lookup.

The package page is dynamic, but the embedded list is public and deterministic;
no browser automation or private API is required. Package definitions are
snapshotted under `data/catalogs/ti-bxl/`, and product queries are checkpointed
after every package. `--max-packages` bounds a run and `--resume` continues it.

The initial live validation found 2,906 package definitions. A bounded run had
processed 120 definitions and accumulated 2,055 product rows, 2,055 direct BXL
links, 1,876 STEP links, and 470 distinct BXL URLs. These are partial-crawl
figures, not portfolio coverage claims.

For a completed crawl interrupted by a network outage, inspect and recover
only failed queries:

```bash
eda-fetch ti-bxl-catalog failures
eda-fetch ti-bxl-catalog retry-failures --transient-only
```

Recovery checkpoints after each package, preserves successful rows, and keeps
remaining transient failures retryable. Use `--max-packages` to bound a retry
batch.
