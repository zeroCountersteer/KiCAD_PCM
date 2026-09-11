use anyhow::Result;
use eda_model::{format_from_filename, is_eda_asset, AssetRecord};
use regex::Regex;
use reqwest::Client;
use url::Url;
pub const DOCUMENTATION_URL: &str = "https://www.analog.com/en/resources/packaging-quality-symbols-footprints/symbols-and-footprints.html";
pub const CATEGORY_URLS: &[&str] = &["https://www.analog.com/en/product-category.html"];
/// Uses official category pages as a bounded product candidate index and retains
/// explicit product URLs from `config/analog-devices-urls.txt` for overrides.
pub async fn discover() -> Result<Vec<AssetRecord>> {
    let client = Client::new();
    let mut pages: Vec<String> = CATEGORY_URLS.iter().map(|x| (*x).into()).collect();
    let path = std::path::Path::new("config/analog-devices-urls.txt");
    if path.exists() {
        pages.extend(
            std::fs::read_to_string(path)?
                .lines()
                .map(str::trim)
                .filter(|x| !x.is_empty() && !x.starts_with('#'))
                .map(str::to_owned),
        );
    }
    let mut candidates = Vec::new();
    let mut category_pages = Vec::new();
    for line in pages {
        let html = client
            .get(&line)
            .header("User-Agent", "eda-library/0.1")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        if line.contains("product-category") {
            candidates.extend(parse_product_candidates(&html, &line)?);
            category_pages.extend(parse_category_candidates(&html, &line)?);
        } else {
            candidates.push(line);
        }
    }
    for page in category_pages.into_iter().take(40) {
        let html = client
            .get(&page)
            .header("User-Agent", "eda-library/0.1")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        candidates.extend(parse_product_candidates(&html, &page)?);
    }
    candidates.sort();
    candidates.dedup();
    let mut out = Vec::new();
    for page in candidates.into_iter().take(100) {
        let html = client
            .get(&page)
            .header("User-Agent", "eda-library/0.1")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        out.extend(parse_product_page(&html, &page)?);
    }
    Ok(out)
}
pub fn parse_product_candidates(html: &str, index_url: &str) -> Result<Vec<String>> {
    let re = Regex::new(r#"(?i)href\s*=\s*[\"']([^\"']+)[\"']"#)?;
    let base = Url::parse(index_url)?;
    Ok(re
        .captures_iter(html)
        .filter_map(|c| {
            let u = base.join(&c[1]).ok()?.to_string();
            (u.contains("/en/products/") || u.contains("/en/product/")).then_some(u)
        })
        .collect())
}
pub fn parse_category_candidates(html: &str, index_url: &str) -> Result<Vec<String>> {
    let re = Regex::new(r#"(?i)href\s*=\s*[\"']([^\"']+)[\"']"#)?;
    let base = Url::parse(index_url)?;
    Ok(re
        .captures_iter(html)
        .filter_map(|c| {
            let u = base.join(&c[1]).ok()?.to_string();
            (u.contains("/en/product-category/")).then_some(u)
        })
        .collect())
}
pub fn parse_product_page(html: &str, part_url: &str) -> Result<Vec<AssetRecord>> {
    let re = Regex::new(r#"(?i)(?:href|src)\s*=\s*[\"']([^\"']+)[\"']"#)?;
    let base = Url::parse(part_url)?;
    let mpn = base
        .path_segments()
        .and_then(|s| s.rev().find(|x| !x.is_empty() && *x != "product"))
        .unwrap_or("unknown")
        .to_string();
    let mut out = Vec::new();
    for cap in re.captures_iter(html) {
        let raw = &cap[1];
        let u = base.join(raw)?;
        let filename = u
            .path_segments()
            .and_then(|mut x| x.next_back())
            .unwrap_or("asset.bin")
            .to_string();
        if is_eda_asset(&filename) {
            out.push(AssetRecord {
                manufacturer: "Analog Devices".into(),
                mpn: mpn.clone(),
                part_url: part_url.into(),
                asset_url: u.to_string(),
                discovery_url: Some(part_url.into()),
                format: format_from_filename(&filename),
                filename,
                content_type: None,
                source_package: None,
                request_url: Some(u.to_string()),
            });
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_fixture() {
        let x = parse_product_page(
            include_str!("../tests/fixtures/product.html"),
            "https://www.analog.com/en/products/ad123.html",
        )
        .unwrap();
        assert_eq!(x.len(), 2);
        assert_eq!(x[0].format, "bxl");
    }
    #[test]
    fn parses_bounded_index_links() {
        let x = parse_product_candidates(
            include_str!("../tests/fixtures/category.html"),
            "https://www.analog.com/en/product-category.html",
        )
        .unwrap();
        let c = parse_category_candidates(
            include_str!("../tests/fixtures/category.html"),
            "https://www.analog.com/en/product-category.html",
        )
        .unwrap();
        assert_eq!(x.len(), 1);
        assert_eq!(c.len(), 1);
    }
}
