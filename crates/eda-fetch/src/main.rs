use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use eda_model::{format_from_filename, AssetRecord, FailureRecord, ManifestRecord};
use reqwest::{header, Client, StatusCode};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::Cursor,
    path::{Path, PathBuf},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    time::{sleep, Duration},
};

#[derive(Parser)]
#[command(
    name = "eda-fetch",
    about = "Conservative manufacturer EDA asset acquisition"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    #[arg(long, global = true, default_value = "data")]
    data: PathBuf,
}
#[derive(Subcommand)]
enum Command {
    Ti {
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        resume: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, default_value_t = 2)]
        concurrency: usize,
        #[arg(long)]
        models_only: bool,
        #[arg(long)]
        dump_discovery: bool,
        #[arg(long)]
        catalog: bool,
        #[arg(long)]
        sample: bool,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        with_step: bool,
        #[arg(long)]
        max_assets: Option<usize>,
    },
    TiCatalog {
        #[command(subcommand)]
        command: TiCatalogCommand,
    },
    TiPublicCatalog {
        #[command(subcommand)]
        command: TiPublicCatalogCommand,
    },
    TiBxlCatalog {
        #[command(subcommand)]
        command: TiBxlCatalogCommand,
    },
    AnalogDevices {
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        resume: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, default_value_t = 2)]
        concurrency: usize,
    },
    Stats,
    List {
        manufacturer: String,
    },
}
#[derive(Subcommand)]
enum TiCatalogCommand {
    Refresh,
    Stats,
    List {
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        contains: Option<String>,
    },
    Sample {
        #[arg(long)]
        limit: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        explain: bool,
    },
}
#[derive(Subcommand)]
enum TiPublicCatalogCommand {
    Refresh {
        #[arg(long)]
        resume: bool,
        #[arg(long)]
        max_duration: Option<String>,
        #[arg(long)]
        max_pages: Option<usize>,
        #[arg(long, default_value_t = 2)]
        concurrency: usize,
    },
    Stats,
    List {
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        package: Option<String>,
        #[arg(long)]
        category: Option<String>,
    },
    Diff,
    Inspect {
        url: String,
    },
    Pending,
}
#[derive(Subcommand)]
enum TiBxlCatalogCommand {
    Refresh {
        #[arg(long)]
        resume: bool,
        #[arg(long)]
        max_packages: Option<usize>,
    },
    Stats,
    Pending,
    RetryFailures {
        #[arg(long)]
        transient_only: bool,
        #[arg(long)]
        max_packages: Option<usize>,
        #[arg(long)]
        verbose: bool,
    },
    Failures {
        #[arg(long)]
        verbose: bool,
    },
    Inspect {
        package: String,
        pin_count: u32,
    },
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CadLookup {
    generic_product: String,
    status: String,
    assets: Vec<AssetRecord>,
    lookup_at: String,
    source_fingerprint: String,
    #[serde(default)]
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Ti {
            limit,
            resume,
            dry_run,
            concurrency,
            models_only,
            dump_discovery,
            catalog,
            sample,
            all,
            with_step,
            max_assets,
        } => {
            run(
                "Texas Instruments",
                "texas-instruments",
                limit,
                resume,
                dry_run,
                concurrency,
                &cli.data,
                models_only,
                dump_discovery,
                if all {
                    Some(load_bxl_catalog(&cli.data).or_else(|_| load_public_catalog(&cli.data))?)
                } else if catalog || sample {
                    let products = load_catalog(&cli.data)?;
                    Some(if sample {
                        let raw = std::fs::read(cli.data.join("catalogs/ti/current.json"))?;
                        vendor_ti::sample_catalog(
                            &products,
                            limit.unwrap_or(products.len()),
                            &format!("{:x}", Sha256::digest(raw)),
                        )
                    } else {
                        products
                    })
                } else {
                    load_catalog(&cli.data).ok()
                },
                with_step,
                max_assets,
            )
            .await?
        }
        Command::AnalogDevices {
            limit,
            resume,
            dry_run,
            concurrency,
        } => {
            run(
                "Analog Devices",
                "analog-devices",
                limit,
                resume,
                dry_run,
                concurrency,
                &cli.data,
                false,
                false,
                None,
                false,
                None,
            )
            .await?
        }
        Command::Stats => stats(&cli.data).await?,
        Command::TiCatalog { command } => match command {
            TiCatalogCommand::Refresh => refresh_catalog(&cli.data).await?,
            TiCatalogCommand::Stats => catalog_stats(&cli.data)?,
            TiCatalogCommand::List { prefix, contains } => {
                catalog_list(&cli.data, prefix.as_deref(), contains.as_deref())?
            }
            TiCatalogCommand::Sample {
                limit,
                json,
                explain,
            } => catalog_sample(&cli.data, limit, json, explain)?,
        },
        Command::TiPublicCatalog { command } => match command {
            TiPublicCatalogCommand::Refresh {
                resume,
                max_duration,
                max_pages,
                concurrency,
            } => {
                refresh_public_catalog(
                    &cli.data,
                    resume,
                    max_duration.as_deref(),
                    max_pages,
                    concurrency,
                )
                .await?
            }
            TiPublicCatalogCommand::Stats => public_catalog_stats(&cli.data)?,
            TiPublicCatalogCommand::List {
                prefix,
                package,
                category,
            } => public_catalog_list(
                &cli.data,
                prefix.as_deref(),
                package.as_deref(),
                category.as_deref(),
            )?,
            TiPublicCatalogCommand::Diff => public_catalog_diff(&cli.data)?,
            TiPublicCatalogCommand::Inspect { url } => {
                public_catalog_inspect(&cli.data, &url).await?
            }
            TiPublicCatalogCommand::Pending => public_catalog_pending(&cli.data)?,
        },
        Command::TiBxlCatalog { command } => match command {
            TiBxlCatalogCommand::Refresh {
                resume,
                max_packages,
            } => refresh_ti_bxl_catalog(&cli.data, resume, max_packages).await?,
            TiBxlCatalogCommand::Stats => ti_bxl_catalog_stats(&cli.data)?,
            TiBxlCatalogCommand::Pending => ti_bxl_catalog_pending(&cli.data)?,
            TiBxlCatalogCommand::RetryFailures {
                transient_only,
                max_packages,
                verbose,
            } => retry_ti_bxl_failures(&cli.data, transient_only, max_packages, verbose).await?,
            TiBxlCatalogCommand::Failures { verbose } => {
                ti_bxl_catalog_failures(&cli.data, verbose)?
            }
            TiBxlCatalogCommand::Inspect { package, pin_count } => println!(
                "TI package query: {} pins {}\nEndpoint: {}",
                package,
                pin_count,
                vendor_ti::PRODUCTS_BY_PACKAGE_URL
            ),
        },
        Command::List { manufacturer } => list(&cli.data, &manufacturer).await?,
    };
    Ok(())
}

async fn discover(kind: &str) -> Result<Vec<AssetRecord>> {
    match kind {
        "texas-instruments" => Ok(vendor_ti::discover().await?),
        "analog-devices" => Ok(vendor_analog_devices::discover().await?),
        _ => anyhow::bail!("unknown vendor"),
    }
}
fn load_catalog(root: &Path) -> Result<Vec<vendor_ti::TiCatalogProduct>> {
    let p = root.join("catalogs/ti/current.json");
    let s = std::fs::read_to_string(p).context("no cached TI catalog; run ti-catalog refresh")?;
    vendor_ti::parse_catalog_json(&s)
}
fn load_public_catalog(root: &Path) -> Result<Vec<vendor_ti::TiCatalogProduct>> {
    let dir = root.join("catalogs/ti-public");
    let p = if dir.join("partial.json").exists() {
        dir.join("partial.json")
    } else {
        dir.join("current.json")
    };
    let s = std::fs::read_to_string(p).context(
        "No public TI catalog snapshot available. Run: eda-fetch ti-public-catalog refresh",
    )?;
    Ok(serde_json::from_str(&s)?)
}
fn load_bxl_catalog(root: &Path) -> Result<Vec<vendor_ti::TiCatalogProduct>> {
    let dir = root.join("catalogs/ti-bxl");
    let path = if dir.join("current.json").exists() {
        dir.join("current.json")
    } else {
        dir.join("partial.json")
    };
    let s = std::fs::read_to_string(path)
        .context("No TI BXL catalog snapshot available. Run: eda-fetch ti-bxl-catalog refresh")?;
    let rows: Vec<vendor_ti::TiPackageProduct> = serde_json::from_str(&s)?;
    Ok(rows
        .into_iter()
        .filter(|r| r.bxl_available != Some(false))
        .map(|r| vendor_ti::TiCatalogProduct {
            ti_part_number: r.part_number.clone(),
            generic_part_number: Some(r.part_number),
            package_type: r.package_code,
            pin_count: r.pin_count,
            lifecycle: r.status,
            product_url: r.product_url,
            category: None,
            discovery_sources: vec![r.source],
            bxl_url: r.bxl_url,
            step_url: r.step_url,
        })
        .collect())
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct TiBxlWork {
    packages: Vec<vendor_ti::TiPackageDefinition>,
    next: usize,
    products: std::collections::BTreeMap<String, vendor_ti::TiPackageProduct>,
    failures: Vec<String>,
    #[serde(default)]
    query_status: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    retry_attempts: std::collections::BTreeMap<String, u32>,
    status: String,
}
async fn refresh_ti_bxl_catalog(
    root: &Path,
    resume: bool,
    max_packages: Option<usize>,
) -> Result<()> {
    let dir = root.join("catalogs/ti-bxl");
    fs::create_dir_all(dir.join("work")).await?;
    let state_path = dir.join("work/state.json");
    let mut work: TiBxlWork = if resume && fs::try_exists(&state_path).await? {
        serde_json::from_str(&fs::read_to_string(&state_path).await?).unwrap_or_default()
    } else {
        TiBxlWork::default()
    };
    let client = Client::builder()
        .user_agent("eda-library/0.1 TI package catalog")
        .timeout(Duration::from_secs(30))
        .build()?;
    if work.packages.is_empty() {
        let body = client
            .get(vendor_ti::PACKAGE_INDEX_URL)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        work.packages = vendor_ti::parse_package_index(&body, vendor_ti::PACKAGE_INDEX_URL);
        if work.packages.is_empty() {
            anyhow::bail!("TI package index contained no package definitions");
        }
        fs::write(
            dir.join("packages.json"),
            serde_json::to_string_pretty(&work.packages)?,
        )
        .await?;
        let package_dir = root.join("catalogs/ti-packages");
        fs::create_dir_all(package_dir.join("snapshots")).await?;
        let package_body = serde_json::to_string_pretty(&work.packages)? + "\n";
        let package_hash = format!("{:x}", Sha256::digest(package_body.as_bytes()));
        atomic_write(&package_dir.join("current.json"), &package_body).await?;
        let package_snapshot = package_dir
            .join("snapshots")
            .join(format!("{package_hash}.json"));
        if !fs::try_exists(&package_snapshot).await? {
            atomic_write(&package_snapshot, &package_body).await?;
        }
        atomic_write(
            &package_dir.join("metadata.json"),
            &serde_json::to_string_pretty(&serde_json::json!({
                "source_type": "ti-public-package-index",
                "source": vendor_ti::PACKAGE_INDEX_URL,
                "sha256": package_hash,
                "record_count": work.packages.len()
            }))?,
        )
        .await?;
    }
    // A crawl may be resumed from work/state created by an older build. Ensure
    // the standalone package-index snapshot is materialized in that case too.
    let package_dir = root.join("catalogs/ti-packages");
    if !fs::try_exists(package_dir.join("current.json")).await? {
        fs::create_dir_all(package_dir.join("snapshots")).await?;
        let package_body = serde_json::to_string_pretty(&work.packages)? + "\n";
        let package_hash = format!("{:x}", Sha256::digest(package_body.as_bytes()));
        atomic_write(&package_dir.join("current.json"), &package_body).await?;
        atomic_write(
            &package_dir.join("metadata.json"),
            &serde_json::to_string_pretty(&serde_json::json!({
                "source_type": "ti-public-package-index",
                "source": vendor_ti::PACKAGE_INDEX_URL,
                "sha256": package_hash,
                "record_count": work.packages.len()
            }))?,
        )
        .await?;
        let snapshot = package_dir
            .join("snapshots")
            .join(format!("{package_hash}.json"));
        if !fs::try_exists(&snapshot).await? {
            atomic_write(&snapshot, &package_body).await?;
        }
    }
    let end = max_packages.map_or(work.packages.len(), |n| {
        (work.next + n).min(work.packages.len())
    });
    while work.next < end {
        let p = &work.packages[work.next];
        let query_key = format!(
            "{}|{}",
            p.package_code,
            p.pin_count.map_or_else(|| "".into(), |n| n.to_string())
        );
        let result = client
            .get(vendor_ti::PRODUCTS_BY_PACKAGE_URL)
            .query(&[
                ("packageDesignator", p.package_code.as_str()),
                ("pinCount", &p.pin_count.unwrap_or_default().to_string()),
                ("results", "results"),
            ])
            .send()
            .await;
        match result {
            Ok(r) => {
                let status = r.status();
                match r.error_for_status() {
                    Ok(r) => {
                        let body = r.text().await?;
                        for mut row in vendor_ti::parse_products_by_package(
                            &body,
                            vendor_ti::PRODUCTS_BY_PACKAGE_URL,
                        ) {
                            row.package_pitch = p.pitch;
                            row.package_height = p.max_height;
                            row.package_length = p.length;
                            row.package_width = p.width;
                            let key = format!(
                                "{}|{}|{}",
                                row.part_number,
                                row.package_code.as_deref().unwrap_or(""),
                                row.pin_count.map_or_else(|| "".into(), |n| n.to_string())
                            );
                            work.products.insert(key, row);
                        }
                        work.query_status.insert(query_key, "Complete".into());
                    }
                    Err(e) => {
                        work.failures.push(format!(
                            "{}: {} (packageDesignator={}&pinCount={})",
                            p.package_code,
                            e,
                            p.package_code,
                            p.pin_count.unwrap_or_default()
                        ));
                        work.query_status.insert(
                            query_key,
                            if transient_http(status) {
                                "FailedTransient"
                            } else {
                                "FailedPermanent"
                            }
                            .into(),
                        );
                    }
                }
            }
            Err(e) => {
                work.failures.push(format!(
                    "{}: {} (packageDesignator={}&pinCount={})",
                    p.package_code,
                    e,
                    p.package_code,
                    p.pin_count.unwrap_or_default()
                ));
                work.query_status
                    .insert(query_key, "FailedTransient".into());
            }
        }
        work.next += 1;
        work.status = if work.next == work.packages.len()
            && !work.query_status.values().any(|s| s == "FailedTransient")
        {
            if work.failures.is_empty() {
                "complete"
            } else {
                "complete_with_permanent_failures"
            }
            .into()
        } else {
            "incomplete".into()
        };
        atomic_write(&state_path, &serde_json::to_string_pretty(&work)?).await?;
        atomic_write(
            &dir.join("partial.json"),
            &serde_json::to_string_pretty(&work.products.values().collect::<Vec<_>>())?,
        )
        .await?;
        println!(
            "TI BXL catalog packages {}/{} products {} failures {}",
            work.next,
            work.packages.len(),
            work.products.len(),
            work.failures.len()
        );
    }
    if work.next == work.packages.len()
        && !work.query_status.values().any(|s| s == "FailedTransient")
    {
        let values = work.products.values().collect::<Vec<_>>();
        let body = serde_json::to_string_pretty(&values)? + "\n";
        atomic_write(&dir.join("current.json"), &body).await?;
        let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
        fs::create_dir_all(dir.join("snapshots")).await?;
        if !fs::try_exists(dir.join("snapshots").join(format!("{hash}.json"))).await? {
            atomic_write(&dir.join("snapshots").join(format!("{hash}.json")), &body).await?;
        }
    }
    Ok(())
}
fn package_failure_key(failure: &str) -> Option<(String, Option<u32>)> {
    let code = failure.split(':').next()?.trim().to_owned();
    let pins = failure
        .split("pinCount=")
        .nth(1)
        .and_then(|x| x.split('&').next())
        .map(|x| {
            x.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|x| x.parse().ok());
    Some((code, pins))
}
fn failure_class(failure: &str) -> &'static str {
    let lower = failure.to_ascii_lowercase();
    if lower.contains("timeout") {
        "Timeout"
    } else if lower.contains("dns") || lower.contains("resolve") {
        "DNS"
    } else if lower.contains("connection")
        || lower.contains("connect")
        || lower.contains("error sending request")
    {
        "Connect"
    } else if lower.contains("429") {
        "HTTP429"
    } else if lower.contains("500") || lower.contains("502") || lower.contains("503") {
        "HTTP5xx"
    } else if lower.contains("http") {
        "HTTP4xx"
    } else {
        "Other"
    }
}
fn transient_http(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status.is_server_error()
}
async fn retry_ti_bxl_failures(
    root: &Path,
    transient_only: bool,
    max_packages: Option<usize>,
    _verbose: bool,
) -> Result<()> {
    let dir = root.join("catalogs/ti-bxl");
    let state_path = dir.join("work/state.json");
    let mut work: TiBxlWork = serde_json::from_str(&fs::read_to_string(&state_path).await?)?;
    let before = work.failures.len();
    let selected = work
        .failures
        .iter()
        .filter(|f| {
            let status = package_failure_key(f).and_then(|(code, pins)| {
                let expected = format!(
                    "{code}|{}",
                    pins.map_or_else(|| "".into(), |n| n.to_string())
                );
                work.query_status.get(&expected).map(String::as_str)
            });
            !status.is_some_and(|value| value == "FailedPermanent")
                && (!transient_only || status == Some("FailedTransient"))
        })
        .take(max_packages.unwrap_or(usize::MAX))
        .cloned()
        .collect::<Vec<_>>();
    let client = Client::builder()
        .user_agent("eda-library/0.1 TI package catalog")
        .timeout(Duration::from_secs(30))
        .build()?;
    let mut recovered = 0usize;
    let mut remaining = work.failures.clone();
    for old_failure in selected {
        let Some((code, pins)) = package_failure_key(&old_failure) else {
            continue;
        };
        let Some(package) = work
            .packages
            .iter()
            .find(|p| p.package_code == code && (pins.is_none() || p.pin_count == pins))
        else {
            continue;
        };
        let query_key = format!(
            "{}|{}",
            code,
            pins.map_or_else(|| "".into(), |n| n.to_string())
        );
        *work.retry_attempts.entry(query_key.clone()).or_default() += 1;
        let mut response = None;
        let mut error = None;
        for attempt in 0..3u64 {
            match client
                .get(vendor_ti::PRODUCTS_BY_PACKAGE_URL)
                .query(&[
                    ("packageDesignator", code.as_str()),
                    ("pinCount", &pins.unwrap_or_default().to_string()),
                    ("results", "results"),
                ])
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    response = Some(r);
                    break;
                }
                Ok(r) => {
                    let status = r.status();
                    error = Some(format!("HTTP {status}"));
                    if !status.is_server_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        break;
                    }
                }
                Err(e) => error = Some(e.to_string()),
            }
            tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))).await;
        }
        if let Some(r) = response {
            let body = r.text().await?;
            for mut row in
                vendor_ti::parse_products_by_package(&body, vendor_ti::PRODUCTS_BY_PACKAGE_URL)
            {
                row.package_pitch = package.pitch;
                row.package_height = package.max_height;
                row.package_length = package.length;
                row.package_width = package.width;
                let key = format!(
                    "{}|{}|{}",
                    row.part_number,
                    row.package_code.as_deref().unwrap_or(""),
                    row.pin_count.map_or_else(|| "".into(), |n| n.to_string())
                );
                work.products.insert(key, row);
            }
            work.query_status.insert(query_key, "Complete".into());
            remaining.retain(|f| f != &old_failure);
            recovered += 1;
        } else if let Some(error) = error {
            let updated = format!(
                "{code}: {error} (packageDesignator={code}&pinCount={})",
                pins.unwrap_or_default()
            );
            remaining.retain(|f| f != &old_failure);
            remaining.push(updated);
            let status = if error.starts_with("HTTP 4") && !error.starts_with("HTTP 429") {
                "FailedPermanent"
            } else {
                "FailedTransient"
            };
            work.query_status.insert(query_key, status.into());
        }
        work.failures = remaining.clone();
        atomic_write(&state_path, &serde_json::to_string_pretty(&work)?).await?;
        atomic_write(
            &dir.join("partial.json"),
            &serde_json::to_string_pretty(&work.products.values().collect::<Vec<_>>())?,
        )
        .await?;
    }
    work.failures = remaining;
    work.status = if work.next == work.packages.len() && work.failures.is_empty() {
        "complete".into()
    } else {
        "incomplete".into()
    };
    if work.status == "complete" {
        let body =
            serde_json::to_string_pretty(&work.products.values().collect::<Vec<_>>())? + "\n";
        atomic_write(&dir.join("current.json"), &body).await?;
    }
    atomic_write(&state_path, &serde_json::to_string_pretty(&work)?).await?;
    println!(
        "TI BXL retry: before {} retried {} recovered {} remaining {}",
        before,
        before.saturating_sub(work.failures.len()),
        recovered,
        work.failures.len()
    );
    Ok(())
}
fn ti_bxl_catalog_stats(root: &Path) -> Result<()> {
    let rows: Vec<vendor_ti::TiPackageProduct> = serde_json::from_str(&std::fs::read_to_string(
        root.join("catalogs/ti-bxl/partial.json"),
    )?)?;
    println!(
        "TI package products {}\nBXL-capable {}\nDirect BXL URLs {}\nSTEP-capable {}",
        rows.len(),
        rows.iter()
            .filter(|r| r.bxl_available != Some(false))
            .count(),
        rows.iter().filter(|r| r.bxl_url.is_some()).count(),
        rows.iter()
            .filter(|r| r.step_available == Some(true))
            .count()
    );
    Ok(())
}
fn ti_bxl_catalog_failures(root: &Path, verbose: bool) -> Result<()> {
    let work: TiBxlWork = serde_json::from_str(&std::fs::read_to_string(
        root.join("catalogs/ti-bxl/work/state.json"),
    )?)?;
    println!(
        "package\tpin_count\tclass\tstatus\tattempts{}",
        if verbose { "\terror" } else { "" }
    );
    for failure in &work.failures {
        let (code, pins) = package_failure_key(failure).unwrap_or_else(|| ("unknown".into(), None));
        let status = work
            .query_status
            .iter()
            .find(|(key, _)| key.starts_with(&format!("{code}|")))
            .map(|(_, value)| value.as_str())
            .unwrap_or("unknown");
        let key = format!(
            "{code}|{}",
            pins.map_or_else(|| "".into(), |n| n.to_string())
        );
        let attempts = work.retry_attempts.get(&key).copied().unwrap_or(0);
        if verbose {
            println!(
                "{code}\t{}\t{}\t{}\t{}\t{}",
                pins.map_or_else(|| "unknown".into(), |n| n.to_string()),
                failure_class(failure),
                status,
                attempts,
                failure
            );
        } else {
            println!(
                "{code}\t{}\t{}\t{}\t{}",
                pins.map_or_else(|| "unknown".into(), |n| n.to_string()),
                failure_class(failure),
                status,
                attempts
            );
        }
    }
    Ok(())
}
fn ti_bxl_catalog_pending(root: &Path) -> Result<()> {
    let w: TiBxlWork = serde_json::from_str(&std::fs::read_to_string(
        root.join("catalogs/ti-bxl/work/state.json"),
    )?)?;
    println!(
        "Pending package definitions {}",
        w.packages.len().saturating_sub(w.next)
    );
    for p in w.packages.iter().skip(w.next).take(20) {
        println!(
            "{}\t{}",
            p.package_code,
            p.pin_count
                .map_or_else(|| "unknown".into(), |n| n.to_string())
        );
    }
    Ok(())
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct PublicWorkItem {
    url: String,
    page_type: String,
    status: String,
    attempts: u32,
    last_error: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct PublicCrawlState {
    status: String,
    queue: Vec<PublicWorkItem>,
    visited: std::collections::BTreeSet<String>,
    processed: usize,
    categories_discovered: usize,
    categories_processed: usize,
    listing_pages_processed: usize,
    transient_failures: usize,
    permanent_failures: usize,
    last_progress: Option<String>,
    pub html_pages: usize,
    pub json_pages: usize,
    pub cached_responses: usize,
    pub network_timeouts: usize,
    pub dns_connect_failures: usize,
    pub http_failures: usize,
    pub products_yielded: usize,
}
async fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).await?;
    fs::rename(&tmp, path).await?;
    Ok(())
}
fn duration_arg(value: Option<&str>) -> Option<std::time::Duration> {
    let v = value?;
    let (n, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
    let n: u64 = n.parse().ok()?;
    Some(match unit {
        "s" => std::time::Duration::from_secs(n),
        "m" => std::time::Duration::from_secs(n * 60),
        "h" => std::time::Duration::from_secs(n * 3600),
        _ => return None,
    })
}
fn public_normalize_url(url: &str) -> String {
    let Ok(mut u) = url::Url::parse(url) else {
        return url.to_owned();
    };
    u.set_fragment(None);
    let kept = u
        .query_pairs()
        .filter(|(k, _)| !k.to_ascii_lowercase().starts_with("utm_") && k != "gclid")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect::<Vec<_>>();
    u.set_query(None);
    if !kept.is_empty() {
        u.query_pairs_mut().extend_pairs(kept);
    }
    u.to_string()
}
async fn refresh_public_catalog(
    root: &Path,
    resume: bool,
    max_duration: Option<&str>,
    max_pages: Option<usize>,
    _concurrency: usize,
) -> Result<()> {
    let client = Client::builder()
        .user_agent("eda-library/0.1 public TI catalog")
        .timeout(Duration::from_secs(30))
        .build()?;
    let dir = root.join("catalogs/ti-public");
    fs::create_dir_all(dir.join("work/responses")).await?;
    let state_path = dir.join("work/state.json");
    let mut state: PublicCrawlState = if resume && fs::try_exists(&state_path).await? {
        serde_json::from_str(&fs::read_to_string(&state_path).await?).unwrap_or_default()
    } else {
        PublicCrawlState {
            status: "incomplete".into(),
            queue: vec![PublicWorkItem {
                url: vendor_ti::PUBLIC_OVERVIEW_URL.into(),
                page_type: "overview".into(),
                status: "Pending".into(),
                attempts: 0,
                last_error: None,
            }],
            ..Default::default()
        }
    };
    let products_path = dir.join("work/products.jsonl");
    let mut products = std::collections::BTreeMap::<String, vendor_ti::TiCatalogProduct>::new();
    if fs::try_exists(&products_path).await? {
        for line in fs::read_to_string(&products_path).await?.lines() {
            if let Ok(p) = serde_json::from_str::<vendor_ti::TiCatalogProduct>(line) {
                products.insert(p.ti_part_number.clone(), p);
            }
        }
    }
    if state.queue.is_empty() && state.status.is_empty() {
        state.status = "incomplete".into();
        state.queue.push(PublicWorkItem {
            url: vendor_ti::PUBLIC_OVERVIEW_URL.into(),
            page_type: "overview".into(),
            status: "Pending".into(),
            attempts: 0,
            last_error: None,
        });
    }
    let start = std::time::Instant::now();
    let pages_at_start = state.processed;
    let budget = match max_duration {
        Some(value) => Some(
            duration_arg(Some(value))
                .ok_or_else(|| anyhow::anyhow!("--max-duration must be like 30s, 10m, or 1h"))?,
        ),
        None => None,
    };
    let mut since_report = std::time::Instant::now();
    while let Some(mut item) = state.queue.pop() {
        if item.status == "Complete" || item.status == "Skipped" {
            continue;
        }
        if item.status == "FailedTransient" && item.attempts >= 3 {
            state.queue.insert(0, item);
            break;
        }
        if max_pages.is_some_and(|n| state.processed.saturating_sub(pages_at_start) >= n)
            || budget.is_some_and(|d| start.elapsed() >= d)
        {
            state.queue.insert(0, item);
            break;
        }
        item.status = "InProgress".into();
        item.attempts += 1;
        let normalized = public_normalize_url(&item.url);
        if state.visited.contains(&normalized) {
            continue;
        }
        let cache = dir
            .join("work/responses")
            .join(format!("{:x}.html", Sha256::digest(normalized.as_bytes())));
        let body = if fs::try_exists(&cache).await? {
            state.cached_responses += 1;
            fs::read_to_string(&cache).await?
        } else {
            let result = match client.get(&normalized).send().await {
                Ok(response) => match response.error_for_status() {
                    Ok(response) => response.text().await,
                    Err(error) => Err(error),
                },
                Err(e) => Err(e),
            };
            match result {
                Ok(body) => {
                    if body.len() > 4 * 1024 * 1024 {
                        item.status = "FailedPermanent".into();
                        item.last_error = Some("catalog response exceeds 4 MiB limit".into());
                        state.http_failures += 1;
                        state.queue.insert(0, item);
                        atomic_write(&state_path, &serde_json::to_string_pretty(&state)?).await?;
                        continue;
                    }
                    atomic_write(&cache, &body).await?;
                    body
                }
                Err(e) => {
                    let transient = e.is_timeout() || e.is_connect() || e.is_request();
                    item.status = if transient {
                        "FailedTransient"
                    } else {
                        "FailedPermanent"
                    }
                    .into();
                    item.last_error = Some(e.to_string());
                    if item.status == "FailedTransient" {
                        state.transient_failures += 1;
                        if e.is_timeout() {
                            state.network_timeouts += 1;
                        }
                        if e.is_connect() {
                            state.dns_connect_failures += 1;
                        }
                    } else {
                        state.permanent_failures += 1;
                        state.http_failures += 1;
                    }
                    if item.attempts < 3 {
                        sleep(Duration::from_millis(250 * 2u64.pow(item.attempts.min(2)))).await;
                        state.queue.insert(0, item);
                    }
                    atomic_write(&state_path, &serde_json::to_string_pretty(&state)?).await?;
                    continue;
                }
            }
        };
        state.visited.insert(normalized.clone());
        state.processed += 1;
        if body.trim_start().starts_with('{') || body.trim_start().starts_with('[') {
            state.json_pages += 1;
        } else {
            state.html_pages += 1;
        }
        if item.page_type == "overview" {
            let links = vendor_ti::parse_public_category_links(&body, &normalized);
            state.categories_discovered += links
                .iter()
                .filter(|x| x.ends_with("/overview.html"))
                .count();
            for url in links {
                let page_type = if url.ends_with("/overview.html") {
                    "overview"
                } else {
                    "listing"
                };
                if !state.visited.contains(&url) {
                    state.queue.push(PublicWorkItem {
                        url,
                        page_type: page_type.into(),
                        status: "Pending".into(),
                        attempts: 0,
                        last_error: None,
                    });
                }
            }
            state.categories_processed += 1;
        } else {
            let (found, next, _) = vendor_ti::parse_public_listing(&body, &normalized);
            state.listing_pages_processed += 1;
            for p in found {
                state.products_yielded += 1;
                products
                    .entry(p.ti_part_number.clone())
                    .and_modify(|old| {
                        for s in &p.discovery_sources {
                            if !old.discovery_sources.contains(s) {
                                old.discovery_sources.push(s.clone());
                            }
                        }
                        old.discovery_sources.sort();
                    })
                    .or_insert(p);
            }
            for url in next {
                let url = public_normalize_url(&url);
                if !state.visited.contains(&url) {
                    state.queue.push(PublicWorkItem {
                        url,
                        page_type: "listing".into(),
                        status: "Pending".into(),
                        attempts: 0,
                        last_error: None,
                    });
                }
            }
        }
        if since_report.elapsed() >= std::time::Duration::from_secs(2) {
            println!("TI public catalog refresh\nPages processed {}\nProducts discovered {}\nPending {}\nFailures {}", state.processed, products.len(), state.queue.len(), state.transient_failures + state.permanent_failures);
            since_report = std::time::Instant::now();
        }
        state.status = "incomplete".into();
        state.last_progress = Some(Utc::now().to_rfc3339());
        atomic_write(&state_path, &serde_json::to_string_pretty(&state)?).await?;
        let text = products
            .values()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?
            .join("\n")
            + if products.is_empty() { "" } else { "\n" };
        atomic_write(&products_path, &text).await?;
    }
    let product_count = products.len();
    if state.queue.is_empty() {
        state.status = "complete".into();
        let values = products.into_values().collect::<Vec<_>>();
        let body = serde_json::to_string_pretty(&values)? + "\n";
        let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
        fs::create_dir_all(dir.join("snapshots")).await?;
        let snapshot = dir.join("snapshots").join(format!("{hash}.json"));
        if !fs::try_exists(&snapshot).await? {
            fs::write(&snapshot, &body).await?;
        }
        atomic_write(&dir.join("current.json"), &body).await?;
        let _ = fs::remove_file(dir.join("partial.json")).await;
        fs::write(dir.join("metadata.json"), serde_json::json!({"source_type":"ti-public-parametric-catalog","status":"complete","response_sha256":hash,"generic_products":values.len(),"pages_processed":state.processed}).to_string()).await?;
    } else {
        atomic_write(
            &dir.join("partial.json"),
            &serde_json::to_string_pretty(&products.values().collect::<Vec<_>>())?,
        )
        .await?;
        fs::write(dir.join("metadata.json"), serde_json::json!({"source_type":"ti-public-parametric-catalog","status":"incomplete","generic_products":product_count,"pages_processed":state.processed,"pending_pages":state.queue.len(),"last_progress":state.last_progress}).to_string()).await?;
    }
    atomic_write(&state_path, &serde_json::to_string_pretty(&state)?).await?;
    println!(
        "TI public catalog refresh status {}\nProducts discovered {}\nPending pages {}",
        state.status,
        product_count,
        state.queue.len()
    );
    Ok(())
}
fn public_catalog_stats(root: &Path) -> Result<()> {
    let p = load_public_catalog(root)?;
    let metadata =
        std::fs::read_to_string(root.join("catalogs/ti-public/metadata.json")).unwrap_or_default();
    let state = std::fs::read_to_string(root.join("catalogs/ti-public/work/state.json"))
        .unwrap_or_default();
    let sources = p
        .iter()
        .flat_map(|x| x.discovery_sources.iter())
        .collect::<std::collections::BTreeSet<_>>();
    println!(
        "Public TI generic products {}\nDiscovery sources {}\nPackage metadata {}\nStatus {}\n{}",
        p.len(),
        sources.len(),
        p.iter().filter(|x| x.package_type.is_some()).count(),
        serde_json::from_str::<serde_json::Value>(&metadata)
            .ok()
            .and_then(|v| v.get("status").and_then(|x| x.as_str()).map(str::to_owned))
            .unwrap_or_else(|| "unknown".into()),
        serde_json::from_str::<PublicCrawlState>(&state).map(|s| format!("Pages processed {}\nPending pages {}\nFailures {}\nHTML pages {}\nJSON pages {}\nCached responses {}\nProducts yielded {}", s.processed, s.queue.len(), s.transient_failures + s.permanent_failures, s.html_pages, s.json_pages, s.cached_responses, s.products_yielded))
            .unwrap_or_default()
    );
    Ok(())
}
async fn public_catalog_inspect(_root: &Path, url: &str) -> Result<()> {
    let started = std::time::Instant::now();
    let client = Client::builder()
        .user_agent("eda-library/0.1 public TI catalog inspector")
        .timeout(Duration::from_secs(30))
        .build()?;
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let categories = vendor_ti::parse_public_category_links(&body, url);
    let (products, next, total) = vendor_ti::parse_public_listing(&body, url);
    let lower = body.to_ascii_lowercase();
    let structured = [
        "__next_data__",
        "initialstate",
        "fetch(",
        "xmlhttprequest",
        "apiurl",
        "endpoint",
        "dataurl",
        "application/json",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    println!("URL {}\nPage type {}\nResponse bytes {}\nResponse time ms {}\nCategory/listing links {}\nProduct links {}\nPagination links {}\nReported total {}\nStructured endpoint/config detected {}", url, if url.ends_with("overview.html") { "category/overview" } else { "parametric listing" }, body.len(), started.elapsed().as_millis(), categories.len(), products.len(), next.len(), total.map_or_else(|| "unknown".into(), |x| x.to_string()), structured);
    Ok(())
}
fn public_catalog_pending(root: &Path) -> Result<()> {
    let s = std::fs::read_to_string(root.join("catalogs/ti-public/work/state.json"))
        .context("no public catalog crawl state")?;
    let state: PublicCrawlState = serde_json::from_str(&s)?;
    println!(
        "Pending pages {}",
        state
            .queue
            .iter()
            .filter(|x| x.status == "Pending" || x.status == "FailedTransient")
            .count()
    );
    for item in state
        .queue
        .iter()
        .filter(|x| x.status == "Pending" || x.status == "FailedTransient")
        .take(20)
    {
        println!("{}\t{}\t{}", item.status, item.page_type, item.url);
    }
    Ok(())
}
fn public_catalog_list(
    root: &Path,
    prefix: Option<&str>,
    package: Option<&str>,
    category: Option<&str>,
) -> Result<()> {
    for p in load_public_catalog(root)?
        .into_iter()
        .filter(|p| prefix.is_none_or(|v| p.ti_part_number.starts_with(v)))
        .filter(|p| {
            package.is_none_or(|v| {
                p.package_type
                    .as_deref()
                    .is_some_and(|x| x.to_ascii_lowercase().contains(&v.to_ascii_lowercase()))
            })
        })
        .filter(|p| {
            category.is_none_or(|v| {
                p.category
                    .as_deref()
                    .is_some_and(|x| x.to_ascii_lowercase().contains(&v.to_ascii_lowercase()))
            })
        })
    {
        println!(
            "{}\t{}",
            p.generic_part_number
                .as_deref()
                .unwrap_or(&p.ti_part_number),
            p.product_url.unwrap_or_default()
        );
    }
    Ok(())
}
fn public_catalog_diff(root: &Path) -> Result<()> {
    let dir = root.join("catalogs/ti-public/snapshots");
    let mut files = std::fs::read_dir(dir)?
        .filter_map(|x| x.ok().map(|y| y.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect::<Vec<_>>();
    files.sort();
    if files.len() < 2 {
        println!("Need at least two public catalog snapshots for diff");
        return Ok(());
    }
    let old: Vec<vendor_ti::TiCatalogProduct> =
        serde_json::from_str(&std::fs::read_to_string(&files[files.len() - 2])?)?;
    let new = load_public_catalog(root)?;
    let a = old
        .iter()
        .map(|p| p.ti_part_number.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let b = new
        .iter()
        .map(|p| p.ti_part_number.clone())
        .collect::<std::collections::BTreeSet<_>>();
    println!(
        "Previous {}\nCurrent {}\nAdded {}\nRemoved {}",
        a.len(),
        b.len(),
        b.difference(&a).count(),
        a.difference(&b).count()
    );
    Ok(())
}
async fn refresh_catalog(root: &Path) -> Result<()> {
    let id = std::env::var("TI_API_CLIENT_ID").context("TI_API_CLIENT_ID is required")?;
    let secret =
        std::env::var("TI_API_CLIENT_SECRET").context("TI_API_CLIENT_SECRET is required")?;
    let client = Client::builder()
        .user_agent("eda-library/0.1 catalog client")
        .timeout(Duration::from_secs(60))
        .build()?;
    let body = vendor_ti::catalog_json(&client, &id, &secret).await?;
    let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    let dir = root.join("catalogs/ti");
    fs::create_dir_all(dir.join("snapshots")).await?;
    fs::write(dir.join("snapshots").join(format!("{hash}.json")), &body).await?;
    fs::write(dir.join("current.json"), &body).await?;
    let products = vendor_ti::parse_catalog_json(&body)?;
    fs::write(dir.join("metadata.json"),serde_json::json!({"retrieved_at":Utc::now(),"endpoint":vendor_ti::STORE_CATALOG_URL,"response_sha256":hash,"record_count":products.len()}).to_string()).await?;
    println!("TI catalog records {}", products.len());
    Ok(())
}
fn catalog_stats(root: &Path) -> Result<()> {
    let p = load_catalog(root)?;
    let g = p
        .iter()
        .filter_map(|x| x.generic_part_number.as_ref())
        .collect::<std::collections::BTreeSet<_>>();
    println!(
        "Catalog orderable products {}\nGeneric products {}",
        p.len(),
        g.len()
    );
    Ok(())
}
fn catalog_list(root: &Path, prefix: Option<&str>, contains: Option<&str>) -> Result<()> {
    for p in load_catalog(root)?
        .into_iter()
        .filter(|x| prefix.is_none_or(|v| x.ti_part_number.starts_with(v)))
        .filter(|x| contains.is_none_or(|v| x.ti_part_number.contains(v)))
    {
        println!(
            "{}\t{}",
            p.ti_part_number,
            p.generic_part_number.unwrap_or_default()
        )
    }
    Ok(())
}
fn catalog_sample(root: &Path, limit: usize, json: bool, explain: bool) -> Result<()> {
    let products = load_catalog(root)?;
    let raw = std::fs::read(root.join("catalogs/ti/current.json"))?;
    let sha = format!("{:x}", Sha256::digest(raw));
    let selected = vendor_ti::sample_catalog(&products, limit, &sha);
    if json {
        println!("{}", serde_json::to_string_pretty(&selected)?);
    } else {
        for p in selected {
            if explain {
                let (category, package, pins, prefix) = vendor_ti::sampling_dimensions(&p);
                println!(
                    "{}\n  category: {}\n  package: {}\n  pins: {}\n  prefix: {}",
                    p.generic_part_number
                        .as_deref()
                        .unwrap_or(&p.ti_part_number),
                    category,
                    package,
                    pins,
                    prefix
                );
            } else {
                println!(
                    "{}",
                    p.generic_part_number
                        .as_deref()
                        .unwrap_or(&p.ti_part_number)
                );
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run(
    _display: &str,
    kind: &str,
    limit: Option<usize>,
    _resume: bool,
    dry: bool,
    concurrency: usize,
    root: &Path,
    models_only: bool,
    dump_discovery: bool,
    catalog: Option<Vec<vendor_ti::TiCatalogProduct>>,
    with_step: bool,
    max_assets: Option<usize>,
) -> Result<()> {
    let mut assets = if let Some(products) = catalog {
        discover_catalog_cached(&products, root, with_step).await?
    } else {
        discover(kind).await?
    };
    if dump_discovery {
        for a in &assets {
            println!(
                "MPN: {}\n  href: {}\n  filename: {}\n  inferred type: {}",
                a.mpn, a.asset_url, a.filename, a.format
            );
        }
    }
    let mut parts = std::collections::BTreeSet::new();
    assets.retain(|a| {
        if kind == "texas-instruments" && !with_step && !models_only && a.format != "bxl" {
            return false;
        }
        if models_only && !matches!(a.format.as_str(), "step" | "stp") {
            return false;
        }
        if let Some(n) = limit {
            if !parts.contains(&a.mpn) && parts.len() >= n {
                return false;
            }
        }
        parts.insert(a.mpn.clone());
        true
    });
    let discovered_assets = assets.len();
    // Keep the many-to-one source relationship separately from download jobs.
    // A URL is downloaded once, but every product/package observation remains
    // queryable for provenance and later canonicalization.
    let mut references = std::collections::BTreeMap::<String, Vec<AssetRecord>>::new();
    for asset in &assets {
        references
            .entry(asset.asset_url.clone())
            .or_default()
            .push(asset.clone());
    }
    let reference_jsonl = references
        .values()
        .map(|items| serde_json::to_string(items).map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?
        .join("\n")
        + if references.is_empty() { "" } else { "\n" };
    let acquisition_dir = root.join("acquisition");
    fs::create_dir_all(&acquisition_dir).await?;
    atomic_write(
        &acquisition_dir.join(format!("{kind}-references.jsonl")),
        &reference_jsonl,
    )
    .await?;
    assets.sort_by(|a, b| a.asset_url.cmp(&b.asset_url).then(a.mpn.cmp(&b.mpn)));
    assets.dedup_by(|a, b| a.asset_url == b.asset_url);
    println!(
        "{} discovery\nAssets discovered: {}\nUnique asset URLs: {}",
        _display,
        discovered_assets,
        assets.len()
    );
    if dry {
        for a in assets {
            println!("{} {}", a.mpn, a.asset_url);
        }
        return Ok(());
    }
    fs::create_dir_all(root.join("objects")).await?;
    fs::create_dir_all(root.join("manifests")).await?;
    fs::create_dir_all(root.join("failures")).await?;
    let client = Client::builder()
        .user_agent("eda-library/0.1 (personal acquisition; contact repository owner)")
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(45))
        .build()?;
    let manifest_path = root.join("manifests").join(format!("{kind}.jsonl"));
    let manifest_known: HashSet<String> = fs::read_to_string(&manifest_path)
        .await
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<ManifestRecord>(l).ok())
        .map(|m| m.asset_url)
        .collect();
    #[derive(serde::Serialize, serde::Deserialize, Default)]
    struct AcquisitionState {
        completed: std::collections::BTreeSet<String>,
        in_progress: std::collections::BTreeSet<String>,
        failures: std::collections::BTreeSet<String>,
    }
    let state_path = root.join("acquisition").join(format!("{kind}.json"));
    fs::create_dir_all(state_path.parent().unwrap()).await?;
    let mut state: AcquisitionState = if _resume && fs::try_exists(&state_path).await? {
        serde_json::from_str(&fs::read_to_string(&state_path).await?).unwrap_or_default()
    } else {
        AcquisitionState::default()
    };
    state.in_progress.clear();
    let known: HashSet<String> = manifest_known
        .into_iter()
        .chain(state.completed.iter().cloned())
        .collect();
    let unique_urls = assets.len();
    let mut pending = assets
        .into_iter()
        .filter(|a| !known.contains(&a.asset_url))
        .collect::<Vec<_>>();
    if let Some(n) = max_assets {
        pending.truncate(n);
    }
    println!("TI BXL acquisition\nCatalog observations: {}\nUnique BXL URLs: {}\nAlready complete: {}\nPending: {}\nTransient failures: {}", discovered_assets, unique_urls, known.len(), pending.len(), state.failures.len());
    let mut manifest = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(manifest_path)
        .await?;
    let mut ok = 0;
    let mut fail = 0;
    let mut jobs = tokio::task::JoinSet::new();
    let mut next = 0usize;
    while next < pending.len() || !jobs.is_empty() {
        while next < pending.len() && jobs.len() < concurrency.max(1) {
            let asset = pending[next].clone();
            next += 1;
            state.in_progress.insert(asset.asset_url.clone());
            let c = client.clone();
            let r = root.to_path_buf();
            jobs.spawn(async move {
                let result = download(&c, &r, &asset).await;
                (asset, result)
            });
        }
        let Some(joined) = jobs.join_next().await else {
            break;
        };
        let (asset, result) = joined?;
        state.in_progress.remove(&asset.asset_url);
        match result {
            Ok(m) => {
                manifest
                    .write_all(serde_json::to_string(&m)?.as_bytes())
                    .await?;
                manifest.write_all(b"\n").await?;
                state.completed.insert(asset.asset_url.clone());
                ok += 1;
            }
            Err(e) => {
                fail += 1;
                let asset_url = asset.asset_url.clone();
                let f = FailureRecord {
                    manufacturer: asset.manufacturer,
                    mpn: asset.mpn,
                    url: asset_url.clone(),
                    stage: "download".into(),
                    http_status: e
                        .to_string()
                        .strip_prefix("HTTP ")
                        .and_then(|s| s.parse().ok()),
                    error: e.to_string(),
                    recorded_at: Utc::now(),
                };
                let p = root.join("failures").join(format!("{kind}.jsonl"));
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .await?;
                file.write_all(format!("{}\n", serde_json::to_string(&f)?).as_bytes())
                    .await?;
                state.failures.insert(asset_url);
            }
        }
        atomic_write(&state_path, &serde_json::to_string_pretty(&state)?).await?;
    }
    println!("Completed: {}; failures: {}", ok, fail);
    Ok(())
}
async fn discover_catalog_cached(
    products: &[vendor_ti::TiCatalogProduct],
    root: &Path,
    with_step: bool,
) -> Result<Vec<AssetRecord>> {
    let path = root.join("catalogs/ti/cad-lookups.jsonl");
    let old = fs::read_to_string(&path).await.unwrap_or_default();
    let mut cache = std::collections::BTreeMap::<String, CadLookup>::new();
    for l in old.lines() {
        if let Ok(x) = serde_json::from_str::<CadLookup>(l) {
            cache.insert(x.generic_product.clone(), x);
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for p in products {
        let generic = p
            .generic_part_number
            .clone()
            .unwrap_or_else(|| p.ti_part_number.clone());
        let package_key = p.package_type.as_deref().unwrap_or("");
        if !seen.insert(format!("{generic}|{package_key}")) {
            continue;
        }
        if let Some(x) = cache.get(&generic) {
            if x.status == "HasCad" {
                out.extend(x.assets.iter().cloned().map(|mut a| {
                    a.mpn = p.ti_part_number.clone();
                    a
                }));
            }
            continue;
        }
        if p.bxl_url.is_some() {
            let mut direct = Vec::new();
            if let Some(url) = &p.bxl_url {
                direct.push(AssetRecord {
                    manufacturer: "Texas Instruments".into(),
                    mpn: p.ti_part_number.clone(),
                    part_url: p.product_url.clone().unwrap_or_default(),
                    asset_url: url.clone(),
                    discovery_url: Some(vendor_ti::PRODUCTS_BY_PACKAGE_URL.into()),
                    filename: url::Url::parse(url)
                        .ok()
                        .and_then(|u| {
                            u.path_segments()
                                .and_then(|mut s| s.next_back())
                                .map(str::to_owned)
                        })
                        .unwrap_or_else(|| format!("{}.bxl", p.ti_part_number)),
                    format: "bxl".into(),
                    content_type: None,
                    source_package: p.package_type.clone(),
                    request_url: Some(url.clone()),
                });
            }
            if with_step {
                if let Some(url) = &p.step_url {
                    direct.push(AssetRecord {
                        manufacturer: "Texas Instruments".into(),
                        mpn: p.ti_part_number.clone(),
                        part_url: p.product_url.clone().unwrap_or_default(),
                        asset_url: url.clone(),
                        discovery_url: Some(vendor_ti::PRODUCTS_BY_PACKAGE_URL.into()),
                        filename: url::Url::parse(url)
                            .ok()
                            .and_then(|u| {
                                u.path_segments()
                                    .and_then(|mut s| s.next_back())
                                    .map(str::to_owned)
                            })
                            .unwrap_or_else(|| format!("{}.step", p.ti_part_number)),
                        format: "step".into(),
                        content_type: None,
                        source_package: p.package_type.clone(),
                        request_url: Some(url.clone()),
                    });
                }
            }
            out.extend(direct);
            continue;
        }
        let assets = match vendor_ti::discover_catalog_product(p).await {
            Ok(assets) => assets,
            Err(error) => {
                eprintln!("TI CAD lookup transient failure for {generic}: {error}");
                // A failed request is not evidence that the product has no CAD.
                // Leave it uncached so a later resume retries it.
                continue;
            }
        };
        let status = if assets.is_empty() { "NoCad" } else { "HasCad" };
        let entry = CadLookup {
            generic_product: generic.clone(),
            status: status.into(),
            assets: assets.clone(),
            lookup_at: Utc::now().to_rfc3339(),
            source_fingerprint: format!(
                "{:x}",
                Sha256::digest(
                    assets
                        .iter()
                        .map(|x| x.asset_url.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            ),
            error: None,
        };
        cache.insert(generic, entry);
        out.extend(assets);
    }
    fs::create_dir_all(path.parent().unwrap()).await?;
    let mut text = String::new();
    for x in cache.values() {
        text.push_str(&format!("{}\n", serde_json::to_string(x)?));
    }
    fs::write(path, text).await?;
    Ok(out)
}

async fn download(client: &Client, root: &Path, asset: &AssetRecord) -> Result<ManifestRecord> {
    let mut last = String::new();
    let mut response = None;
    for attempt in 0..3 {
        match client.get(&asset.asset_url).send().await {
            Ok(r) if r.status().is_success() => {
                response = Some(r);
                break;
            }
            Ok(r)
                if matches!(
                    r.status(),
                    StatusCode::TOO_MANY_REQUESTS
                        | StatusCode::REQUEST_TIMEOUT
                        | StatusCode::INTERNAL_SERVER_ERROR
                        | StatusCode::BAD_GATEWAY
                        | StatusCode::SERVICE_UNAVAILABLE
                        | StatusCode::GATEWAY_TIMEOUT
                ) =>
            {
                last = r.status().to_string();
                sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await
            }
            Ok(r) => anyhow::bail!("HTTP {}", r.status()),
            Err(e) => {
                last = e.to_string();
                sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await
            }
        }
    }
    let r = match response {
        Some(r) => r,
        None => anyhow::bail!("request failed after retries: {last}"),
    };
    let etag = r
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let lm = r
        .headers()
        .get(header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let ct = r
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let disposition = r
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(';').find_map(|x| {
                x.trim()
                    .strip_prefix("filename=")
                    .map(|x| x.trim_matches('"').to_string())
            })
        });
    let filename = disposition
        .clone()
        .unwrap_or_else(|| asset.filename.clone());
    let final_url = r.url().to_string();
    let tmp = root.join(format!(
        ".download-{:x}.tmp",
        Sha256::digest(asset.asset_url.as_bytes())
    ));
    let mut f = fs::File::create(&tmp).await?;
    let mut hasher = Sha256::new();
    let mut size = 0;
    let mut stream = r;
    while let Some(chunk) = stream.chunk().await? {
        hasher.update(&chunk);
        size += chunk.len() as u64;
        f.write_all(&chunk).await?;
    }
    f.flush().await?;
    let validation_bytes = if filename.to_ascii_lowercase().ends_with(".zip") {
        fs::read(&tmp).await?
    } else {
        let mut check = fs::File::open(&tmp).await?;
        let mut prefix = vec![0u8; 4096];
        let n = check.read(&mut prefix).await?;
        prefix.truncate(n);
        prefix
    };
    if let Err(error) = validate_content(&filename, ct.as_deref(), &validation_bytes) {
        let _ = fs::remove_file(&tmp).await;
        return Err(error);
    }
    let hash = format!("{:x}", hasher.finalize());
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let object = object_path(root, &hash, &ext);
    fs::create_dir_all(object.parent().unwrap()).await?;
    if fs::try_exists(&object).await? {
        fs::remove_file(&tmp).await?;
    } else {
        fs::rename(&tmp, &object).await?;
    }
    Ok(ManifestRecord {
        manufacturer: asset.manufacturer.clone(),
        mpn: asset.mpn.clone(),
        part_url: asset.part_url.clone(),
        asset_url: asset.asset_url.clone(),
        discovery_url: asset.discovery_url.clone(),
        filename: asset.filename.clone(),
        format: format_from_filename(&filename),
        sha256: hash,
        size,
        retrieved_at: Utc::now(),
        http_etag: etag,
        http_last_modified: lm,
        content_type: ct,
        source_package: asset.source_package.clone(),
        request_url: Some(asset.asset_url.clone()),
        final_download_url: Some(final_url),
        content_disposition_filename: disposition,
    })
}

fn object_path(root: &Path, sha256: &str, format: &str) -> PathBuf {
    let normalized = format.trim_start_matches('.').to_ascii_lowercase();
    root.join("objects")
        .join(&sha256[..2])
        .join(format!("{sha256}.{normalized}"))
}

fn validate_content(filename: &str, content_type: Option<&str>, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        anyhow::bail!("zero-length response")
    }
    let sample = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    if sample.contains("<html")
        || sample.contains("<!doctype html")
        || sample.contains("access denied")
        || sample.contains("captcha")
    {
        anyhow::bail!("HTML/error page returned for {filename}")
    }
    if sample.trim_start().starts_with('{')
        && (sample.contains("error") || sample.contains("message"))
    {
        anyhow::bail!("JSON error response returned for {filename}")
    }
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".zip")
        && !bytes.starts_with(b"PK\x03\x04")
        && !bytes.starts_with(b"PK\x05\x06")
    {
        anyhow::bail!("invalid ZIP signature")
    }
    if lower.ends_with(".zip") {
        zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|_| anyhow::anyhow!("ZIP archive cannot be opened"))?;
    }
    if (lower.ends_with(".stp") || lower.ends_with(".step")) && !sample.contains("iso-10303") {
        anyhow::bail!("response is not recognizable STEP text")
    }
    if lower.ends_with(".bxl") && content_type.is_some_and(|x| x.contains("text/html")) {
        anyhow::bail!("BXL returned as HTML")
    }
    Ok(())
}

async fn list(root: &Path, manufacturer: &str) -> Result<()> {
    let p = root.join("manifests").join(format!("{manufacturer}.jsonl"));
    if let Ok(s) = fs::read_to_string(p).await {
        print!("{s}");
    }
    Ok(())
}
async fn stats(root: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(root.join("manifests"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
    {
        let s = std::fs::read_to_string(entry.path())?;
        let mut n = 0;
        let mut bytes = 0;
        let mut formats = std::collections::BTreeMap::<String, usize>::new();
        for line in s.lines() {
            if let Ok(m) = serde_json::from_str::<ManifestRecord>(line) {
                n += 1;
                bytes += m.size;
                *formats.entry(m.format).or_default() += 1;
            }
        }
        println!(
            "{}: assets={}, bytes={}, formats={:?}",
            entry.file_name().to_string_lossy(),
            n,
            bytes,
            formats
        );
    }
    let refs = root.join("acquisition/texas-instruments-references.jsonl");
    let observations = std::fs::read_to_string(&refs)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Vec<AssetRecord>>(line).ok())
        .map(|items| items.len())
        .sum::<usize>();
    let unique_urls = std::fs::read_to_string(&refs)
        .unwrap_or_default()
        .lines()
        .count();
    let state_path = root.join("acquisition/texas-instruments.json");
    if let Ok(text) = std::fs::read_to_string(state_path) {
        if let Ok(state) = serde_json::from_str::<serde_json::Value>(&text) {
            let manifest_urls =
                std::fs::read_to_string(root.join("manifests/texas-instruments.jsonl"))
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| serde_json::from_str::<ManifestRecord>(line).ok())
                    .map(|m| m.asset_url)
                    .collect::<HashSet<_>>();
            println!(
                "TI BXL catalog: observations={} unique_urls={}\nTI acquisition: complete_urls={} pending_or_failed={}\nRaw storage: unique_objects={} bytes={}",
                observations,
                unique_urls,
                manifest_urls.len(),
                state.get("failures").and_then(|x| x.as_array()).map_or(0, Vec::len),
                walkdir::WalkDir::new(root.join("objects")).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()).count(),
                walkdir::WalkDir::new(root.join("objects")).into_iter().filter_map(Result::ok).filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum::<u64>()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        duration_arg, failure_class, package_failure_key, public_normalize_url, validate_content,
    };
    #[test]
    fn rejects_error_pages_and_bad_signatures() {
        assert!(validate_content("x.bxl", Some("text/html"), b"<!doctype html>error").is_err());
        assert!(validate_content("x.stp", Some("text/plain"), b"not a step").is_err());
        assert!(validate_content("x.zip", None, b"not zip").is_err());
        assert!(validate_content("x.bxl", None, b"BXL\0payload").is_ok());
        assert!(validate_content("x.stp", None, b"ISO-10303-21;\nEND-ISO-10303-21;").is_ok());
    }
    #[test]
    fn catalog_budget_and_url_normalization_are_bounded() {
        assert_eq!(duration_arg(Some("10m")).unwrap().as_secs(), 600);
        assert!(duration_arg(Some("10x")).is_none());
        assert_eq!(
            public_normalize_url("https://ti.test/a?page=2&utm_source=x#top"),
            "https://ti.test/a?page=2"
        );
    }
    #[test]
    fn package_failures_keep_retry_class_and_identity() {
        let failure = "DGG: error sending request (packageDesignator=DGG&pinCount=48)";
        assert_eq!(package_failure_key(failure), Some(("DGG".into(), Some(48))));
        assert_eq!(failure_class("DGG: HTTP 503"), "HTTP5xx");
        assert_eq!(failure_class("DGG: error sending request"), "Connect");
        assert_eq!(failure_class("DGG: timeout"), "Timeout");
    }
}
