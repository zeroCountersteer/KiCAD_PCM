# Public TI discovery

The public discovery backend starts at TI's official product overview
(`https://www.ti.com/product-category/overview.html`), follows official
category overview and parametric `products.html` links, and follows pagination
links exposed by those listings. The previously tried
`https://www.ti.com/sitemap.xml` returned HTTP 404 during live validation and
is no longer used. The backend does not crawl arbitrary pages, use the Store
API, or query Ultra Librarian.

Refresh and inspect the immutable normalized snapshot with:

```bash
eda-fetch ti-public-catalog refresh
eda-fetch ti-public-catalog stats
eda-fetch ti-public-catalog list --prefix TPS
eda-fetch ti-public-catalog diff
eda-fetch ti-public-catalog pending
eda-fetch ti-public-catalog inspect https://www.ti.com/product-category/amplifiers/products.html
```

Refresh is incremental and writes crawl state after each page. Use bounded
runs while testing, then resume them:

```bash
eda-fetch ti-public-catalog refresh --max-pages 100
eda-fetch ti-public-catalog refresh --resume --max-pages 100
eda-fetch ti-public-catalog refresh --max-duration 10m --resume
```

The active frontier and cached responses are under
`data/catalogs/ti-public/work/`. Partial products are available in
`partial.json`; a completed crawl alone updates `current.json` and creates an
immutable content-addressed snapshot. The last completed snapshot is not
overwritten by an incomplete crawl.

The snapshot is stored under `data/catalogs/ti-public/`. Its records represent
generic product identities where the public listings expose them; package, category,
pin-count, lifecycle, and orderable aliases remain unknown unless the public
index provides them. The tool reports this limitation rather than inventing
metadata.

Acquire all products in the cached public snapshot using BXL-only behavior:

```bash
eda-fetch ti --all --resume
```

Use `--with-step` explicitly to include STEP/STP assets. Existing authenticated
catalog and prefix discovery remain separate fallback/debug mechanisms.
