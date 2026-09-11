# TI live sample report

Date: 2026-09-10

Command attempted: `./target/release/eda-fetch ti --limit 2 --dry-run`.

The first run against the old catalog failed with HTTP 404. The adapter was then updated to the live public form flow. A 10-asset run completed successfully, followed by the requested 100-asset run. The manifest currently contains 200 records because the second run was safely repeatable but appended duplicate manifest observations; it contains 100 unique URLs/hashes and 48 inferred MPNs. Two records are the `ULib.zip` reader installer from the pre-filter run, not component CAD assets; future runs exclude it.

| Metric | Result |
|---|---:|
| Parts requested / discovered / processed | 100 assets / 48 inferred MPNs / 100 unique URLs |
| Assets discovered / downloaded | 100 unique / 100 unique |
| Unique objects / duplicates | 100 / 100 duplicate manifest observations |
| BXL / STEP-STP / ZIP | 198 / 3 / 2 (reader installer) |
| Other formats / bytes | 0 / 246,607,386 |
| Failed requests | 1 obsolete catalog request (404) |
| Unexpected MIME / zero-length / collisions | 0 / 0 / 0 |

The previous zero-STEP result was caused by applying `--limit` to asset URLs and deriving the MPN of package-level files such as `D0014A.stp` from the STEP filename. The corrected adapter limits unique parts and carries the current MPN from each BXL/STEP row. A live two-part validation acquired 3 STEP files with no failures. Run locally with `./target/release/eda-fetch ti --limit 100 --concurrency 2` for a larger paired sample.
