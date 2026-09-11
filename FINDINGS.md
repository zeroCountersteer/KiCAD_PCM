# Discovery findings

Research date: 2026-09-10.

* Texas Instruments: `pf_cad_data.txt` now returns 404. The live landing page still exposes a normal public form posting `partno` to `/cad/cad.cgi`; responses contain direct BXL and STP links under `/cad/dlbxl.cgi/`. No JSON API, pagination contract, or stable bulk-query contract was established. The adapter uses a bounded prefix list and excludes the 111 MB `ULib.zip` reader installer. The live run acquired 10 assets successfully, and a subsequent 100-asset attempt was interrupted by the execution time limit after writing duplicate manifest records; see `reports/ti-sample.md`.
* TI STEP capture: the links were present in the same HTML rows, but the old run truncated asset URLs before reaching STEP links and package-level names such as `D0014A.stp` were misinterpreted as MPNs. The adapter now limits unique parts, retains paired assets, supports `--models-only`, and records request/final URLs plus Content-Disposition when available. A live two-part run acquired 3 valid STP objects.
* Analog Devices / Maxim: official product-category pages provide a bounded candidate index, then product pages expose direct CAD links when present. The adapter starts from the official category page, follows at most 40 category pages and inspects at most 100 product candidates, plus explicit URLs in `config/analog-devices-urls.txt`. It does not crawl Ultra Librarian or arbitrary analog.com links.

No component library was downloaded into Git, and no conversion or archive rewriting is performed.

* TI catalog: the official authenticated Store Inventory and Pricing API v2 full catalog endpoint is used as an explicit cached backend. Its documented limit is one call per four hours; no automatic refresh is performed. Prefix discovery remains the credential-free fallback.
## TI package-driven BXL discovery

TI's public packaging pages provide a more useful credential-free discovery
path than the slow category listings. `searchalltipackages.tsp` embeds a
public JSON package list, and `searchproductbypackage.tsp` accepts
`packageDesignator` and `pinCount`. Product rows expose direct WEBENCH BXL
links (and often STEP links), so the acquisition layer can avoid redundant
`cad.cgi` lookups. The implementation stores package definitions and product
rows in resumable `data/catalogs/ti-bxl` state. A bounded live validation
observed 2,906 package definitions and, after 120 processed definitions,
2,055 product rows with 470 distinct BXL URLs. The crawl was incomplete; these
numbers are not a claim about the full TI portfolio.
