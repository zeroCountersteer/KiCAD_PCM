# TI package index

Source: `https://www.ti.com/packaging/docs/searchalltipackages.tsp`.

The live page returns HTTP 200 and embeds the package table as a JavaScript JSON
`list`; the parser extracts package family (`pType`), designator (`dgn`), pin
count (`pnC`), pitch (`pit`), maximum height (`hgt`), length (`BL`), and width
(`BW`). The embedded list yielded 2,906 distinct `(package code, pin count)`
definitions in the validation run.

## Bounded validation

All 2,906 package definitions have been visited. The finalized index contains
35,574 product rows, 35,572 direct BXL links, and 31,026 STEP links. The BXL URL
set contains 8,566 unique URLs, demonstrating substantial package-asset reuse.
All 131 transient package-query failures recovered successfully.
