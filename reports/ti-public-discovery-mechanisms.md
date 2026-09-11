# TI public discovery mechanisms

The supported credential-free mechanism is the official TI product-category
graph:

```text
https://www.ti.com/product-category/overview.html
    -> category overview pages
    -> category products.html parametric listings
    -> pagination links exposed by each listing
```

The crawler accepts server-rendered product links and explicit pagination links.
It records reported totals when listing text contains an `of N` count and
caches page responses for offline parser development. Pages that report a
total but contain no product links remain diagnosable through
`eda-fetch ti-public-catalog inspect URL`; the crawler does not invent a
private XHR endpoint or add browser automation.

The previously attempted `https://www.ti.com/sitemap.xml` returned HTTP 404 and
is not used. Live coverage remains an observed property of the public listing
graph, not a claim that it equals TI's complete portfolio.
