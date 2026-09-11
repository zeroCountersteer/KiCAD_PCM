# Analog Devices adapter

Analog Devices documents CAD models on product pages and links to Ultra Librarian/Samacsys for broader search. The adapter starts with the official product-category page, follows a bounded set of category links, and inspects at most 100 resulting product pages. Put explicit product URLs in `config/analog-devices-urls.txt` for overrides/debugging. Only direct CAD/EDA links in returned HTML are normalized; Ultra Librarian itself is never crawled.
