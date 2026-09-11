# TI public catalog discovery

The implemented no-credential discovery source is TI's public product-category
overview and linked parametric `products.html` listings. The previously tried
`https://www.ti.com/sitemap.xml` returned HTTP 404 during live validation and
is not used.

Live refresh was not run in this environment, so no coverage numbers are
fabricated here. Run `eda-fetch ti-public-catalog refresh` and then `stats` to
populate the local snapshot and obtain observed counts. Listing result caps,
missing package metadata, and products not offering WEBENCH CAD must be
reported from that run.

The full acquisition default is BXL-only; STEP/STP requires `--with-step`.
