use anyhow::Result;
use eda_model::{format_from_filename, AssetRecord};
use regex::Regex;
use reqwest::Client;
use sha2::{Digest, Sha256};
use url::Url;
pub const SAMPLER_VERSION: &str = "ti-stratified-v1";
pub const CATALOG_URL: &str = "https://webench.ti.com/cad/";
pub const SEARCH_URL: &str = "https://webench.ti.com/cad/cad.cgi";
pub const STORE_CATALOG_URL: &str = "https://transact.ti.com/v2/store/products/catalog";
pub const OAUTH_URL: &str = "https://transact.ti.com/v1/oauth/accesstoken";
pub const PUBLIC_OVERVIEW_URL: &str = "https://www.ti.com/product-category/overview.html";
pub const PACKAGE_INDEX_URL: &str = "https://www.ti.com/packaging/docs/searchalltipackages.tsp";
pub const PRODUCTS_BY_PACKAGE_URL: &str =
    "https://www.ti.com/packaging/docs/searchproductbypackage.tsp";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CadAssetKind {
    Bxl,
    Step,
    Zip,
    Unknown,
}

/// Classify only the asset filename.  CGI endpoint names such as `dlbxl.cgi`
/// and `newstep` are intentionally ignored.
pub fn classify_cad_url(value: &str) -> CadAssetKind {
    let filename = Url::parse(value)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find_map(|(key, value)| {
                    (key.eq_ignore_ascii_case("filename") || key.eq_ignore_ascii_case("file"))
                        .then(|| value.into_owned())
                })
                .or_else(|| {
                    url.path_segments()
                        .and_then(|mut s| s.next_back())
                        .map(str::to_owned)
                })
        })
        .or_else(|| {
            let path = value.split('?').next()?.split('#').next()?;
            path.rsplit('/').next().map(str::to_owned)
        });
    match filename
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .rsplit('.')
        .next()
    {
        Some("bxl") => CadAssetKind::Bxl,
        Some("stp") | Some("step") => CadAssetKind::Step,
        Some("zip") => CadAssetKind::Zip,
        _ => CadAssetKind::Unknown,
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct TiCatalogProduct {
    pub ti_part_number: String,
    pub generic_part_number: Option<String>,
    pub package_type: Option<String>,
    pub pin_count: Option<u32>,
    pub lifecycle: Option<String>,
    pub product_url: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub discovery_sources: Vec<String>,
    #[serde(default)]
    pub bxl_url: Option<String>,
    #[serde(default)]
    pub step_url: Option<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TiGenericProduct {
    pub generic_part_number: String,
    pub representative: TiCatalogProduct,
    pub orderable_parts: Vec<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct TiPackageDefinition {
    pub package_family: Option<String>,
    pub package_code: String,
    pub pin_count: Option<u32>,
    pub pitch: Option<f64>,
    pub max_height: Option<f64>,
    pub length: Option<f64>,
    pub width: Option<f64>,
    pub source: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct TiPackageProduct {
    pub part_number: String,
    pub description: Option<String>,
    pub functionality: Option<String>,
    pub status: Option<String>,
    pub automotive: Option<String>,
    pub package_code: Option<String>,
    pub pin_count: Option<u32>,
    pub bxl_available: Option<bool>,
    pub step_available: Option<bool>,
    pub product_url: Option<String>,
    pub bxl_url: Option<String>,
    pub step_url: Option<String>,
    #[serde(default)]
    pub package_pitch: Option<f64>,
    #[serde(default)]
    pub package_height: Option<f64>,
    #[serde(default)]
    pub package_length: Option<f64>,
    #[serde(default)]
    pub package_width: Option<f64>,
    pub source: String,
}

fn cell_text(s: &str) -> String {
    Regex::new(r"(?is)<[^>]+>")
        .map(|r| r.replace_all(s, " ").into_owned())
        .unwrap_or_else(|_| s.to_owned())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "yes" | "y" | "true" | "available" | "x" => Some(true),
        "no" | "n" | "false" | "none" | "-" => Some(false),
        _ => None,
    }
}
fn parse_num(s: &str) -> Option<f64> {
    s.trim()
        .replace(',', ".")
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn package_from_json(value: &serde_json::Value, source: &str, out: &mut Vec<TiPackageDefinition>) {
    if let Some(list) = value.get("list").and_then(serde_json::Value::as_array) {
        for item in list {
            let text = |key: &str| {
                item.get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let code = text("dgn").unwrap_or_default();
            if code.is_empty() {
                continue;
            }
            out.push(TiPackageDefinition {
                package_family: text("pType"),
                package_code: code,
                pin_count: text("pnC").and_then(|v| v.parse().ok()),
                pitch: text("pit").and_then(|v| parse_num(&v)),
                max_height: text("hgt").and_then(|v| parse_num(&v)),
                length: text("BL").and_then(|v| parse_num(&v)),
                width: text("BW").and_then(|v| parse_num(&v)),
                source: source.to_owned(),
            });
        }
    }
}

fn embedded_json_array(body: &str) -> Option<&str> {
    let start = body.find("\"list\"")?;
    let open = body[start..].find('[')? + start;
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (i, byte) in bytes.iter().enumerate().skip(open) {
        if quoted {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'\"' {
                quoted = false;
            }
            continue;
        }
        match *byte {
            b'\"' => quoted = true,
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return body.get(open..=i);
                }
            }
            _ => {}
        }
    }
    None
}
pub fn parse_package_index(body: &str, source: &str) -> Vec<TiPackageDefinition> {
    let mut out = Vec::new();
    if let Some(array) = embedded_json_array(body) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(array) {
            package_from_json(&serde_json::json!({"list": value}), source, &mut out);
        }
    }
    if out.is_empty() {
        let row_re = Regex::new(r"(?is)<tr[^>]*>(.*?)</tr>").expect("valid regex");
        let cell_re = Regex::new(r"(?is)<t[dh][^>]*>(.*?)</t[dh]>").expect("valid regex");
        for row in row_re.captures_iter(body) {
            let cells = cell_re
                .captures_iter(&row[1])
                .map(|c| cell_text(&c[1]))
                .collect::<Vec<_>>();
            if cells.len() >= 2
                && !cells[0].to_ascii_lowercase().contains("package")
                && cells[0].chars().any(|c| c.is_ascii_alphanumeric())
            {
                out.push(TiPackageDefinition {
                    package_family: cells.first().cloned(),
                    package_code: cells.get(1).cloned().unwrap_or_default(),
                    pin_count: cells.get(2).and_then(|x| x.parse().ok()),
                    pitch: cells.get(3).and_then(|x| parse_num(x)),
                    max_height: cells.get(4).and_then(|x| parse_num(x)),
                    length: cells.get(5).and_then(|x| parse_num(x)),
                    width: cells.get(6).and_then(|x| parse_num(x)),
                    source: source.to_owned(),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        a.package_code
            .cmp(&b.package_code)
            .then(a.pin_count.cmp(&b.pin_count))
    });
    out.dedup_by(|a, b| a.package_code == b.package_code && a.pin_count == b.pin_count);
    out
}
pub fn parse_products_by_package(body: &str, source: &str) -> Vec<TiPackageProduct> {
    let row_re = Regex::new(r"(?is)<tr[^>]*>(.*?)</tr>").expect("valid regex");
    let cell_re = Regex::new(r"(?is)<t[dh][^>]*>(.*?)</t[dh]>").expect("valid regex");
    let link_re = Regex::new(r#"(?is)<a[^>]+href\s*=\s*[\"']([^\"']+)[\"']"#).expect("valid regex");
    let mut out = Vec::new();
    for row in row_re.captures_iter(body) {
        let cells = cell_re
            .captures_iter(&row[1])
            .map(|c| cell_text(&c[1]))
            .collect::<Vec<_>>();
        if cells.is_empty() || cells[0].to_ascii_lowercase().contains("part number") {
            continue;
        }
        let links = link_re
            .captures_iter(&row[1])
            .map(|c| {
                let href = c[1].to_owned();
                if href.starts_with("//") {
                    format!("https:{href}")
                } else {
                    href
                }
            })
            .collect::<Vec<_>>();
        let part = cells[0].clone();
        if part.is_empty() || part.len() > 80 {
            continue;
        }
        let bxl_url = links
            .iter()
            .find(|x| classify_cad_url(x) == CadAssetKind::Bxl)
            .cloned();
        let step_url = links
            .iter()
            .find(|x| classify_cad_url(x) == CadAssetKind::Step)
            .cloned();
        let bxl_available = bxl_url.is_some().then_some(true).or_else(|| {
            cells.iter().find_map(|x| {
                if x.to_ascii_lowercase().contains("bxl") {
                    parse_bool(x)
                } else {
                    None
                }
            })
        });
        let step_available = step_url.is_some().then_some(true).or_else(|| {
            cells.iter().find_map(|x| {
                let lower = x.to_ascii_lowercase();
                if lower.contains("step") || lower.contains("stp") {
                    parse_bool(x)
                } else {
                    None
                }
            })
        });
        out.push(TiPackageProduct {
            part_number: part,
            description: cells.get(1).cloned(),
            functionality: cells.get(2).cloned(),
            status: cells.get(4).cloned(),
            automotive: cells.get(3).cloned(),
            package_code: cells
                .get(5)
                .map(|x| x.split('|').next().unwrap_or(x).to_owned()),
            pin_count: cells
                .get(5)
                .and_then(|x| x.split('|').nth(1).and_then(|v| v.trim().parse().ok())),
            bxl_available,
            step_available,
            product_url: links.first().cloned(),
            bxl_url,
            step_url,
            package_pitch: None,
            package_height: None,
            package_length: None,
            package_width: None,
            source: source.to_owned(),
        });
    }
    out
}

pub fn parse_public_category_links(body: &str, base_url: &str) -> Vec<String> {
    let Ok(base) = Url::parse(base_url) else {
        return Vec::new();
    };
    let Ok(re) = Regex::new(r#"(?is)<a[^>]+href\s*=\s*[\"']([^\"']+)[\"']"#) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for c in re.captures_iter(body) {
        let Ok(url) = base.join(c[1].trim()) else {
            continue;
        };
        let path = url.path();
        if path.contains("/product-category/")
            && (path.ends_with("/overview.html") || path.ends_with("/products.html"))
            && path != "/product-category/overview.html"
        {
            out.push(url.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn parse_public_listing(
    body: &str,
    source: &str,
) -> (Vec<TiCatalogProduct>, Vec<String>, Option<usize>) {
    let mut out = Vec::new();
    let Ok(base) = Url::parse(source) else {
        return (out, Vec::new(), None);
    };
    let Ok(re) = Regex::new(r#"(?is)<a[^>]+href\s*=\s*[\"']([^\"']+)[\"'][^>]*>(.*?)</a>"#) else {
        return (out, Vec::new(), None);
    };
    for c in re.captures_iter(body) {
        let Ok(parsed) = base.join(c[1].trim()) else {
            continue;
        };
        let url = parsed.to_string();
        let Ok(parsed) = Url::parse(&url) else {
            continue;
        };
        let Some(mut segments) = parsed.path_segments() else {
            continue;
        };
        if segments.next() != Some("product") {
            continue;
        }
        let Some(generic) = segments.next() else {
            continue;
        };
        if generic.is_empty() || generic.eq_ignore_ascii_case("part-details") {
            continue;
        }
        out.push(TiCatalogProduct {
            ti_part_number: generic.to_owned(),
            generic_part_number: Some(generic.to_owned()),
            package_type: None,
            pin_count: None,
            lifecycle: None,
            product_url: Some(url),
            category: None,
            discovery_sources: vec![source.to_owned()],
            bxl_url: None,
            step_url: None,
        });
    }
    let total = Regex::new(r"(?i)\bof\s+(\d[\d,]*)").ok().and_then(|r| {
        r.captures(body)
            .and_then(|c| c[1].replace(',', "").parse().ok())
    });
    let mut next = Vec::new();
    for c in re.captures_iter(body) {
        let text = c[2].to_ascii_lowercase();
        if text.contains("next")
            || text.contains('›')
            || c[1].to_ascii_lowercase().contains("page=")
        {
            if let Ok(url) = base.join(c[1].trim()) {
                next.push(url.to_string());
            }
        }
    }
    next.sort();
    next.dedup();
    (out, next, total)
}

pub fn merge_public_products(
    sources: Vec<(String, Vec<TiCatalogProduct>)>,
) -> Vec<TiCatalogProduct> {
    let mut merged = std::collections::BTreeMap::<String, TiCatalogProduct>::new();
    for (source, products) in sources {
        for mut p in products {
            let key = p
                .generic_part_number
                .clone()
                .unwrap_or_else(|| p.ti_part_number.clone());
            let entry = merged.entry(key.clone()).or_insert_with(|| {
                p.generic_part_number = Some(key.clone());
                p
            });
            if !entry.discovery_sources.iter().any(|x| x == &source) {
                entry.discovery_sources.push(source.clone());
            }
            entry.discovery_sources.sort();
        }
    }
    merged.into_values().collect()
}

pub async fn discover_public_catalog(client: &Client) -> Result<Vec<TiCatalogProduct>> {
    let overview = client
        .get(PUBLIC_OVERVIEW_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut pending = parse_public_category_links(&overview, PUBLIC_OVERVIEW_URL);
    let mut sources = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    while let Some(url) = pending.pop() {
        if !visited.insert(url.clone()) || visited.len() > 500 {
            continue;
        }
        let body = client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let listings = if url.ends_with("/overview.html") {
            parse_public_category_links(&body, &url)
        } else {
            vec![url.clone()]
        };
        for listing in listings {
            if visited.contains(&listing) {
                continue;
            }
            let listing_body = client
                .get(&listing)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let (products, next, _) = parse_public_listing(&listing_body, &listing);
            sources.push((listing, products));
            for page in next {
                if !visited.contains(&page) {
                    pending.push(page);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    Ok(merge_public_products(sources))
}

pub fn generic_products(products: &[TiCatalogProduct]) -> Vec<TiGenericProduct> {
    let mut groups = std::collections::BTreeMap::<String, Vec<TiCatalogProduct>>::new();
    for p in products {
        let key = p
            .generic_part_number
            .clone()
            .unwrap_or_else(|| p.ti_part_number.clone());
        groups.entry(key).or_default().push(p.clone());
    }
    groups
        .into_iter()
        .map(|(generic, mut members)| {
            members.sort_by(|a, b| a.ti_part_number.cmp(&b.ti_part_number));
            let mut representative = members[0].clone();
            representative.generic_part_number = Some(generic.clone());
            for p in &members[1..] {
                if representative.package_type.is_none() {
                    representative.package_type = p.package_type.clone();
                }
                if representative.pin_count.is_none() {
                    representative.pin_count = p.pin_count;
                }
                if representative.lifecycle.is_none() {
                    representative.lifecycle = p.lifecycle.clone();
                }
                if representative.product_url.is_none() {
                    representative.product_url = p.product_url.clone();
                }
                if representative.category.is_none() {
                    representative.category = p.category.clone();
                }
            }
            TiGenericProduct {
                generic_part_number: generic,
                representative,
                orderable_parts: members.into_iter().map(|p| p.ti_part_number).collect(),
            }
        })
        .collect()
}

fn pin_bucket(pin_count: Option<u32>) -> &'static str {
    match pin_count {
        Some(1..=8) => "1-8",
        Some(9..=16) => "9-16",
        Some(17..=32) => "17-32",
        Some(33..=64) => "33-64",
        Some(65..=128) => "65-128",
        Some(129..=256) => "129-256",
        Some(_) => "257+",
        None => "unknown",
    }
}
fn prefix(part: &str) -> String {
    let upper = part.to_ascii_uppercase();
    let mut end = 0;
    let mut digits = 0;
    for (i, c) in upper.char_indices() {
        if c.is_ascii_alphabetic() || (c.is_ascii_digit() && digits < 2) {
            if c.is_ascii_digit() {
                digits += 1;
            }
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let value = if end == 0 { "UNKNOWN" } else { &upper[..end] };
    value.chars().take(6).collect()
}
pub fn sampling_dimensions(p: &TiCatalogProduct) -> (String, String, String, String) {
    (
        p.category.clone().unwrap_or_else(|| "unknown".into()),
        p.package_type.clone().unwrap_or_else(|| "unknown".into()),
        pin_bucket(p.pin_count).into(),
        prefix(
            p.generic_part_number
                .as_deref()
                .unwrap_or(&p.ti_part_number),
        ),
    )
}
pub fn sample_catalog(
    products: &[TiCatalogProduct],
    limit: usize,
    catalog_sha: &str,
) -> Vec<TiCatalogProduct> {
    if limit == 0 {
        return Vec::new();
    }
    let generics = generic_products(products);
    let primary = if generics.iter().any(|g| g.representative.category.is_some()) {
        0
    } else if generics
        .iter()
        .any(|g| g.representative.package_type.is_some())
    {
        1
    } else if generics
        .iter()
        .any(|g| g.representative.pin_count.is_some())
    {
        2
    } else {
        3
    };
    let mut groups = std::collections::BTreeMap::<String, Vec<TiGenericProduct>>::new();
    for g in generics {
        let dims = sampling_dimensions(&g.representative);
        let key = match primary {
            0 => dims.0,
            1 => dims.1,
            2 => dims.2,
            _ => dims.3,
        };
        groups.entry(key).or_default().push(g);
    }
    let mut all = Vec::new();
    for (category, mut group) in groups {
        group.sort_by_key(|g| {
            let d = sampling_dimensions(&g.representative);
            stable_sample_key(
                catalog_sha,
                &g.generic_part_number,
                &format!(
                    "{}:{}:{}:{}:{}",
                    category, d.1, d.2, d.3, g.generic_part_number
                ),
            )
        });
        all.push(group);
    }
    let mut selected = Vec::new();
    let mut round = 0;
    while selected.len() < limit {
        let mut added = false;
        for group in &all {
            if let Some(g) = group.get(round) {
                selected.push(g.representative.clone());
                added = true;
                if selected.len() == limit {
                    break;
                }
            }
        }
        if !added {
            break;
        }
        round += 1;
    }
    selected
}
fn stable_sample_key(catalog_sha: &str, generic: &str, category: &str) -> String {
    format!(
        "{:x}:{}:{}",
        Sha256::digest(format!("{catalog_sha}:{SAMPLER_VERSION}:{category}:{generic}").as_bytes()),
        generic,
        category
    )
}
pub fn parse_catalog_json(body: &str) -> Result<Vec<TiCatalogProduct>> {
    let v: serde_json::Value = serde_json::from_str(body)?;
    let a = v
        .as_array()
        .cloned()
        .or_else(|| v.get("products").and_then(|x| x.as_array()).cloned())
        .or_else(|| v.get("catalog").and_then(|x| x.as_array()).cloned())
        .ok_or_else(|| anyhow::anyhow!("catalog response is not an array or products object"))?;
    let mut out = Vec::new();
    for x in a {
        let get = |k: &str| x.get(k).and_then(|z| z.as_str()).map(str::to_owned);
        let get_first = |keys: &[&str]| keys.iter().find_map(|k| get(k));
        if let Some(t) = get("tiPartNumber").or_else(|| get("ti_part_number")) {
            out.push(TiCatalogProduct {
                ti_part_number: t,
                generic_part_number: get("genericPartNumber")
                    .or_else(|| get("generic_part_number")),
                package_type: get("packageType"),
                pin_count: x.get("pinCount").and_then(|z| z.as_u64()).map(|z| z as u32),
                lifecycle: get("lifeCycle").or_else(|| get("lifecycle")),
                product_url: get("buyNowURL").or_else(|| get("productUrl")),
                category: get_first(&["category", "productCategory", "family", "productFamily"]),
                discovery_sources: Vec::new(),
                bxl_url: None,
                step_url: None,
            });
        }
    }
    out.sort_by(|a, b| {
        a.generic_part_number
            .cmp(&b.generic_part_number)
            .then(a.ti_part_number.cmp(&b.ti_part_number))
    });
    Ok(out)
}
pub async fn catalog_json(client: &Client, id: &str, secret: &str) -> Result<String> {
    let token_body = client
        .post(OAUTH_URL)
        .basic_auth(id, Some(secret))
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let token_json: serde_json::Value = serde_json::from_str(&token_body)?;
    let token = token_json
        .get("access_token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("OAuth response missing access_token"))?
        .to_string();
    Ok(client
        .get(STORE_CATALOG_URL)
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}
pub async fn discover() -> Result<Vec<AssetRecord>> {
    let prefixes = std::fs::read_to_string("config/ti-prefixes.txt")
        .unwrap_or_else(|_| "TPS6\nLM3\nTLV\nOPA\nINA\nUCC\nDRV\nSN74\n".into());
    let client = Client::builder().user_agent("eda-library/0.1").build()?;
    let mut out = Vec::new();
    for prefix in prefixes
        .lines()
        .map(str::trim)
        .filter(|x| x.len() >= 4 && !x.starts_with('#'))
    {
        let body = client
            .post(SEARCH_URL)
            .form(&[("partno", prefix)])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        out.extend(parse_search_response(&body, SEARCH_URL)?);
    }
    out.sort_by(|a, b| a.asset_url.cmp(&b.asset_url));
    out.dedup_by(|a, b| a.asset_url == b.asset_url);
    Ok(out)
}
pub async fn discover_catalog_products(products: &[TiCatalogProduct]) -> Result<Vec<AssetRecord>> {
    let client = Client::builder().user_agent("eda-library/0.1").build()?;
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for p in products {
        let query = p
            .generic_part_number
            .as_deref()
            .unwrap_or(&p.ti_part_number);
        if !seen.insert(query.to_string()) {
            continue;
        }
        let body = client
            .post(SEARCH_URL)
            .form(&[("partno", query)])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        for mut a in parse_search_response(&body, SEARCH_URL)? {
            a.mpn = p.ti_part_number.clone();
            a.part_url = p.product_url.clone().unwrap_or_else(|| {
                format!(
                    "https://www.ti.com/product/{}",
                    p.generic_part_number
                        .as_deref()
                        .unwrap_or(&p.ti_part_number)
                )
            });
            out.push(a)
        }
    }
    out.sort_by(|a, b| a.mpn.cmp(&b.mpn).then(a.asset_url.cmp(&b.asset_url)));
    out.dedup_by(|a, b| a.mpn == b.mpn && a.asset_url == b.asset_url);
    Ok(out)
}
pub async fn discover_catalog_product(p: &TiCatalogProduct) -> Result<Vec<AssetRecord>> {
    let client = Client::builder().user_agent("eda-library/0.1").build()?;
    let query = p
        .generic_part_number
        .as_deref()
        .unwrap_or(&p.ti_part_number);
    let body = client
        .post(SEARCH_URL)
        .form(&[("partno", query)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut out = parse_search_response(&body, SEARCH_URL)?;
    for a in &mut out {
        a.mpn = p.ti_part_number.clone();
        a.part_url = p.product_url.clone().unwrap_or_else(|| {
            format!(
                "https://www.ti.com/product/{}",
                p.generic_part_number
                    .as_deref()
                    .unwrap_or(&p.ti_part_number)
            )
        });
    }
    Ok(out)
}
pub fn parse_search_response(body: &str, search_url: &str) -> Result<Vec<AssetRecord>> {
    let re = Regex::new(
        r#"(?i)href\s*=\s*[\"']([^\"']+(?:\.bxl|\.stp|\.step|\.zip)(?:\?[^\"']*)?)[\"']"#,
    )?;
    let base = Url::parse(search_url)?;
    let mut out = Vec::new();
    let mut current_mpn: Option<String> = None;
    let mut current_package: Option<String> = None;
    for c in re.captures_iter(body) {
        let href = c[1].replace("&amp;", "&");
        let candidate = href.to_ascii_lowercase();
        if !candidate.contains(".bxl")
            && !candidate.contains(".stp")
            && !candidate.contains(".step")
            && !candidate.contains(".zip")
        {
            continue;
        }
        let u = base.join(&href)?;
        if u.path().ends_with("/ULib.zip") {
            continue;
        }
        let filename = u
            .path_segments()
            .and_then(|mut x| x.next_back())
            .unwrap_or("asset.bin")
            .to_string();
        let filename = if filename == "cad.cgi" || !filename.contains('.') {
            u.query_pairs()
                .find(|(k, _)| k.eq_ignore_ascii_case("filename") || k.eq_ignore_ascii_case("file"))
                .map(|(_, v)| v.into_owned())
                .unwrap_or(filename)
        } else {
            filename
        };
        let is_model = filename.to_ascii_lowercase().ends_with(".stp")
            || filename.to_ascii_lowercase().ends_with(".step");
        let mpn = if is_model {
            current_mpn
                .clone()
                .unwrap_or_else(|| filename.split('_').next().unwrap_or(&filename).to_string())
        } else {
            filename.split('_').next().unwrap_or(&filename).to_string()
        };
        if !is_model {
            current_mpn = Some(mpn.clone());
            current_package = filename
                .split('_')
                .nth(2)
                .map(|x| x.trim_end_matches(&['.', 'b', 'x', 'l'][..]).to_string());
        }
        out.push(AssetRecord {
            manufacturer: "Texas Instruments".into(),
            mpn: mpn.clone(),
            part_url: format!("https://www.ti.com/product/{mpn}"),
            asset_url: u.to_string(),
            discovery_url: Some(search_url.into()),
            format: format_from_filename(&filename),
            filename: filename.clone(),
            content_type: None,
            source_package: if is_model {
                current_package.clone()
            } else {
                filename
                    .split('_')
                    .nth(2)
                    .map(|x| x.trim_end_matches(&['.', 'b', 'x', 'l'][..]).to_string())
            },
            request_url: Some(u.to_string()),
        });
    }
    Ok(out)
}
pub fn parse_catalog(body: &str) -> Result<Vec<AssetRecord>> {
    let re = Regex::new(r#"(?i)(https?://[^\s\"']+\.(?:bxl|stp?|zip)(?:\?[^\s\"']*)?)"#)?;
    let mut out = Vec::new();
    for m in re.find_iter(body) {
        let u = m
            .as_str()
            .trim_end_matches(&[')', ',', ';'][..])
            .to_string();
        let filename = Url::parse(&u)?
            .path_segments()
            .and_then(|mut x| x.next_back())
            .unwrap_or("asset.bin")
            .to_string();
        if filename.eq_ignore_ascii_case("ULib.zip") {
            continue;
        }
        let mpn = filename.split('_').next().unwrap_or(&filename).to_string();
        out.push(AssetRecord {
            manufacturer: "Texas Instruments".into(),
            mpn: mpn.clone(),
            part_url: format!("https://www.ti.com/product/{mpn}"),
            asset_url: u.clone(),
            discovery_url: Some(CATALOG_URL.into()),
            format: format_from_filename(&filename),
            filename,
            content_type: None,
            source_package: None,
            request_url: Some(u.clone()),
        });
    }
    out.sort_by(|a, b| a.asset_url.cmp(&b.asset_url));
    out.dedup_by(|a, b| a.asset_url == b.asset_url);
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_fixture() {
        let x = parse_catalog(include_str!("../tests/fixtures/catalog.txt")).unwrap();
        assert_eq!(x.len(), 2);
        assert!(x.iter().any(|a| a.format == "bxl"));
    }
    #[test]
    fn parses_catalog_fixture() {
        let x = parse_catalog_json(include_str!("../tests/fixtures/catalog.json")).unwrap();
        assert_eq!(x.len(), 5);
        assert!(x
            .iter()
            .any(|p| p.generic_part_number.as_deref() == Some("SN74HC00")));
        assert!(x.iter().any(|p| p.category.as_deref() == Some("Logic")));
    }
    fn product(
        mpn: &str,
        generic: &str,
        category: Option<&str>,
        package: Option<&str>,
        pins: Option<u32>,
    ) -> TiCatalogProduct {
        TiCatalogProduct {
            ti_part_number: mpn.into(),
            generic_part_number: Some(generic.into()),
            package_type: package.map(str::to_owned),
            pin_count: pins,
            lifecycle: None,
            product_url: None,
            category: category.map(str::to_owned),
            discovery_sources: Vec::new(),
            bxl_url: None,
            step_url: None,
        }
    }
    #[test]
    fn sampling_is_deterministic_balanced_and_alias_preserving() {
        let input = vec![
            product("TPS1A", "TPS1", Some("Power"), Some("QFN"), Some(32)),
            product("TPS1B", "TPS1", Some("Power"), Some("QFN"), Some(32)),
            product("SN74A", "SN74A", Some("Logic"), Some("SOIC"), Some(14)),
            product("MSP1", "MSP1", Some("MCU"), Some("BGA"), Some(100)),
            product("OPA1", "OPA1", Some("Amplifier"), None, None),
        ];
        let a = sample_catalog(&input, 4, "catalog-sha");
        let b = sample_catalog(
            &input.into_iter().rev().collect::<Vec<_>>(),
            4,
            "catalog-sha",
        );
        assert_eq!(
            a.iter().map(|p| &p.ti_part_number).collect::<Vec<_>>(),
            b.iter().map(|p| &p.ti_part_number).collect::<Vec<_>>()
        );
        assert_eq!(
            generic_products(&[
                product("TPS1A", "TPS1", None, None, None),
                product("TPS1B", "TPS1", None, None, None)
            ])[0]
                .orderable_parts
                .len(),
            2
        );
        assert_eq!(sample_catalog(&a, 20, "catalog-sha").len(), 4);
    }
    #[test]
    fn sampling_changes_with_catalog_identity_and_exposes_dimensions() {
        let p = product(
            "SN74HC00DR",
            "SN74HC00",
            Some("Logic"),
            Some("SOIC"),
            Some(14),
        );
        assert_eq!(sampling_dimensions(&p).2, "9-16");
        assert_ne!(
            sample_catalog(std::slice::from_ref(&p), 1, "a")[0].ti_part_number,
            ""
        );
        assert_eq!(prefix("SN74HC00"), "SN74HC");
    }
    #[test]
    fn parses_live_form_shape() {
        let x = parse_search_response(r#"<a href="/cad/dlbxl.cgi/TI_BXL/TPS62840DLCR.bxl">BXL</a><a href="/cad/dlbxl.cgi/newstep/VSON0010.stp">STEP</a>"#, SEARCH_URL).unwrap();
        assert_eq!(x.len(), 2);
        assert_eq!(x[0].discovery_url.as_deref(), Some(SEARCH_URL));
        let y = parse_search_response(r#"<a href="/cad/dlbxl.cgi/TI_BXL/SN7400_D_14.bxl" target=_blank>SN7400_D_14.bxl</a></td><td><a href="/cad/dlbxl.cgi/newstep/D0014A.stp" target=_blank>D0014A.stp</a>"#, SEARCH_URL).unwrap();
        assert!(y.iter().any(|a| a.format == "stp"), "{y:?}");
    }
    #[test]
    fn classifies_ti_cad_urls_by_filename_only() {
        assert_eq!(
            classify_cad_url("https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/FOO.bxl"),
            CadAssetKind::Bxl
        );
        assert_eq!(
            classify_cad_url("https://webench.ti.com/cad/dlbxl.cgi/newstep/D0014A.stp"),
            CadAssetKind::Step
        );
        assert_eq!(
            classify_cad_url("https://example.test/download?filename=FOO.STEP"),
            CadAssetKind::Step
        );
        assert_eq!(
            classify_cad_url("https://example.test/dlbxl.cgi/unknown"),
            CadAssetKind::Unknown
        );
    }
    #[test]
    fn parses_dynamic_package_index_data() {
        let html = r#"<script>var json_parsed={"list":[{"dgn":"DGG","pType":"Plastic Small Outline","pnC":"48","pit":"0.5","hgt":"1.2","BL":"12.5","BW":"6.1"}]};</script>"#;
        let packages = parse_package_index(html, "fixture");
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_code, "DGG");
        assert_eq!(packages[0].pin_count, Some(48));
        assert_eq!(packages[0].length, Some(12.5));
    }
    #[test]
    fn parses_package_rows_and_direct_cad_links() {
        let html = r#"<table><tr><th>Part number</th></tr>
          <tr><td><a href="//www.ti.com/product/FOO">FOO.B</a></td><td>desc</td><td>logic</td><td>N</td><td>ACTIVE</td><td>DGG|48</td>
            <td><a href="https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/FOO_DGG_48.bxl">BXL</a></td>
            <td><a href="https://webench.ti.com/cad/dlbxl.cgi/newstep/DGG0048A.stp">STEP</a></td></tr>
          <tr><td>BAR.A</td><td>desc</td><td>logic</td><td>N</td><td>ACTIVE</td><td>D14|14</td><td>-</td><td>-</td></tr></table>"#;
        let rows = parse_products_by_package(html, "fixture");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].part_number, "FOO.B");
        assert_eq!(
            rows[0].product_url.as_deref(),
            Some("https://www.ti.com/product/FOO")
        );
        assert_eq!(rows[0].package_code.as_deref(), Some("DGG"));
        assert_eq!(rows[0].pin_count, Some(48));
        assert_eq!(rows[0].bxl_available, Some(true));
        assert_eq!(rows[0].step_available, Some(true));
        assert!(rows[0]
            .bxl_url
            .as_deref()
            .is_some_and(|url| classify_cad_url(url) == CadAssetKind::Bxl));
        assert!(rows[0]
            .step_url
            .as_deref()
            .is_some_and(|url| classify_cad_url(url) == CadAssetKind::Step));
        assert_eq!(rows[1].bxl_available, None);
    }
    #[test]
    fn step_only_row_never_becomes_bxl() {
        let html = r#"<table><tr><td>ONLYSTEP</td><td>d</td><td>f</td><td>N</td><td>ACTIVE</td><td>D14|14</td>
            <td><a href="https://webench.ti.com/cad/dlbxl.cgi/newstep/D0014A.stp">model</a></td></tr></table>"#;
        let rows = parse_products_by_package(html, "fixture");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bxl_url, None);
        assert_eq!(
            rows[0].step_url.as_deref(),
            Some("https://webench.ti.com/cad/dlbxl.cgi/newstep/D0014A.stp")
        );
    }
    #[test]
    fn parses_and_merges_public_listing_products() {
        let html = r#"<a href="/product/TPS62840">TPS62840</a><a href="/product/TPS62840/part-details/TPS62840DLCR">TPS62840DLCR</a><a rel="next" href="products.html?page=2">Next</a><span>1 to 50 of 702</span>"#;
        let (a, next, total) = parse_public_listing(
            html,
            "https://www.ti.com/product-category/power/products.html",
        );
        let (b, _, _) = parse_public_listing(
            html,
            "https://www.ti.com/product-category/power/products.html",
        );
        assert_eq!(a.len(), 2);
        assert_eq!(next.len(), 1);
        assert_eq!(total, Some(702));
        let merged = merge_public_products(vec![("category".into(), a), ("sitemap".into(), b)]);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].discovery_sources.iter().any(|x| x == "category"));
        assert!(merged[0].discovery_sources.iter().any(|x| x == "sitemap"));
        assert!(merged[0]
            .discovery_sources
            .iter()
            .any(|x| x.contains("products.html")));
    }
}
