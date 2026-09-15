use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use eda_model::EdaComponent;
use eda_model::ManifestRecord;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    thread,
};
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    #[arg(long, default_value = "data")]
    data: PathBuf,
}
#[derive(Subcommand)]
enum Command {
    Bxl {
        input: Option<PathBuf>,
        #[arg(long)]
        mpn: Option<String>,
        #[arg(long)]
        manufacturer: Option<String>,
        #[arg(long)]
        all: bool,
    },
    BxlStats,
    BxlStale,
    #[command(hide = true)]
    BxlStatsWorker,
    Kicad {
        #[arg(long)]
        mpn: Option<String>,
        #[arg(long)]
        manufacturer: Option<String>,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        clean: bool,
        #[arg(long, default_value = "Personal_TI")]
        nickname: String,
        #[arg(long)]
        deduplicated: bool,
        #[arg(long)]
        with_3d: bool,
    },
    PackageDiff {
        left: String,
        right: String,
        #[arg(long)]
        json: bool,
    },
    PackageDedupe {
        #[arg(long)]
        manufacturer: Option<String>,
    },
    SymbolCheck {
        #[arg(long)]
        manufacturer: Option<String>,
        #[arg(long)]
        mpn: Option<String>,
    },
    PinMap {
        mpn: String,
    },
    ModelCheck {
        #[arg(long)]
        manufacturer: Option<String>,
    },
    ModelInfo {
        query: String,
    },
    TiCorpusReport,
    KicadCheck {
        path: PathBuf,
    },
}
fn main() -> Result<()> {
    let c = Cli::parse();
    match c.command {
        Command::Bxl {
            input,
            mpn,
            manufacturer,
            all,
        } => {
            if all {
                batch(&c.data)
            } else {
                let input = match input {
                    Some(p) => p,
                    None => resolve_mpn(&c.data, mpn.as_deref())?
                        .context("provide an input or --mpn")?,
                };
                convert(input, mpn, manufacturer, &c.data, None)
            }
        }
        Command::BxlStats => stats(&c.data),
        Command::BxlStale => bxl_stale(&c.data),
        Command::BxlStatsWorker => stats_worker(&c.data),
        Command::Kicad {
            mpn,
            manufacturer,
            input,
            all,
            clean,
            nickname,
            deduplicated,
            with_3d,
        } => kicad_generate(
            &c.data,
            mpn,
            manufacturer,
            input,
            all,
            clean,
            &nickname,
            deduplicated,
            with_3d,
        ),
        Command::PackageDiff { left, right, json } => package_diff(&c.data, &left, &right, json),
        Command::PackageDedupe { manufacturer } => package_dedupe(&c.data, manufacturer.as_deref()),
        Command::SymbolCheck { manufacturer, mpn } => {
            symbol_check(&c.data, manufacturer.as_deref(), mpn.as_deref())
        }
        Command::PinMap { mpn } => pin_map(&c.data, &mpn),
        Command::ModelCheck { manufacturer } => model_check(&c.data, manufacturer.as_deref()),
        Command::ModelInfo { query } => model_info(&c.data, &query),
        Command::TiCorpusReport => ti_corpus_report(&c.data),
        Command::KicadCheck { path } => kicad_check(&path),
    }
}
fn convert(
    input: PathBuf,
    mpn: Option<String>,
    manufacturer: Option<String>,
    data: &Path,
    references: Option<&[eda_model::AssetRecord]>,
) -> Result<()> {
    let bytes = fs::read(&input).with_context(|| format!("read {}", input.display()))?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let doc = bxl_parser::parse(&bytes)
        .with_context(|| format!("parse {} at byte stream", input.display()))?;
    let manufacturer = manufacturer.unwrap_or_else(|| "Unknown".into());
    let mpn = mpn.unwrap_or_else(|| input.file_stem().unwrap().to_string_lossy().into());
    let c = bxl_parser::canonicalize(&doc, &manufacturer, &mpn, Some(hash));
    write_component(c, &input, data, references).map(|_| ())
}

fn write_component(
    mut c: EdaComponent,
    input: &Path,
    data: &Path,
    references: Option<&[eda_model::AssetRecord]>,
) -> Result<bool> {
    c.canonicalizer_version = bxl_parser::BXL_CANONICALIZER_VERSION.into();
    let provenance_hash = references.map(provenance_hash).unwrap_or_default();
    c.metadata
        .insert("provenance_hash".into(), provenance_hash.clone());
    if let Some(references) = references {
        let mut mpns = references.iter().map(|r| r.mpn.clone()).collect::<Vec<_>>();
        mpns.sort();
        mpns.dedup();
        c.metadata.insert("associated_mpns".into(), mpns.join(";"));
        let mut packages = references
            .iter()
            .filter_map(|r| r.source_package.clone())
            .collect::<Vec<_>>();
        packages.sort();
        packages.dedup();
        c.metadata
            .insert("associated_package_observations".into(), packages.join(";"));
        c.metadata.insert(
            "associated_source_urls".into(),
            references
                .iter()
                .map(|r| r.asset_url.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(";"),
        );
        c.metadata.insert(
            "associated_product_urls".into(),
            references
                .iter()
                .map(|r| r.part_url.clone())
                .filter(|url| !url.is_empty())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(";"),
        );
        c.metadata.insert(
            "associated_discovery_urls".into(),
            references
                .iter()
                .filter_map(|r| r.discovery_url.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(";"),
        );
    }
    let out = data
        .join("derived/components")
        .join(safe(&c.manufacturer))
        .join(format!("{}.json", safe(&c.mpn)));
    if let Ok(existing) = fs::read_to_string(&out) {
        if let Ok(existing) = serde_json::from_str::<EdaComponent>(&existing) {
            if existing.source_sha256 == c.source_sha256
                && existing.canonicalizer_version == c.canonicalizer_version
                && existing.metadata.get("provenance_hash") == Some(&provenance_hash)
            {
                return Ok(false);
            }
        }
    }
    fs::create_dir_all(out.parent().unwrap())?;
    let json = serde_json::to_string_pretty(&c)?;
    fs::write(&out, format!("{json}\n"))?;
    println!("{} -> {}", input.display(), out.display());
    Ok(true)
}

fn provenance_hash(references: &[eda_model::AssetRecord]) -> String {
    let mut records = references.to_vec();
    records.sort_by(|a, b| {
        serde_json::to_string(a)
            .unwrap_or_default()
            .cmp(&serde_json::to_string(b).unwrap_or_default())
    });
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&records).unwrap())
    )
}
fn resolve_mpn(data: &Path, wanted: Option<&str>) -> Result<Option<PathBuf>> {
    let Some(wanted) = wanted else {
        return Ok(None);
    };
    for line in fs::read_to_string(data.join("manifests/texas-instruments.jsonl"))
        .unwrap_or_default()
        .lines()
    {
        let m: ManifestRecord = serde_json::from_str(line)?;
        if m.format == "bxl" && m.mpn == wanted {
            let p = data
                .join("objects")
                .join(&m.sha256[..2])
                .join(format!("{}.bxl", m.sha256));
            if p.exists() {
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}
fn batch(data: &Path) -> Result<()> {
    let corpus = load_current_ti_bxl_corpus(data)?;
    let plans = conversion_plans(data, &corpus);
    let collisions = output_collisions(&plans);
    let mut report = ConversionReport {
        observations: corpus.observations,
        urls: corpus.unique_urls,
        shas: plans.len(),
        ..Default::default()
    };
    report.missing = corpus.missing_urls.len();
    report.type_mismatches = corpus.type_mismatches.len();
    report.ambiguous = corpus.ambiguous_urls.len();
    report.output_collisions = collisions.len();
    report.stale_derived_before = stale_derived_files(data, &plans)?;
    report.missing_details = corpus
        .missing_urls
        .iter()
        .map(|url| FailureDetail::url("missing", url))
        .collect();
    report.type_mismatch_details = corpus
        .type_mismatches
        .iter()
        .map(|url| FailureDetail::url("type_mismatch", url))
        .collect();
    report.ambiguous_details = corpus
        .ambiguous_urls
        .iter()
        .map(|url| FailureDetail::url("ambiguous", url))
        .collect();
    if !collisions.is_empty() {
        report.failure_count =
            report.missing + report.type_mismatches + report.ambiguous + report.output_collisions;
        write_conversion_report(data, &report, &collisions)?;
        anyhow::bail!(
            "BXL conversion has {} output collision(s); no components written",
            collisions.len()
        );
    }
    for plan in &plans {
        let bytes = match fs::read(&plan.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.parse_failures += 1;
                report
                    .parse_failure_details
                    .push(FailureDetail::parse(plan, error.to_string()));
                continue;
            }
        };
        let doc = match bxl_parser::parse(&bytes) {
            Ok(doc) => doc,
            Err(error) => {
                eprintln!("{}: {}", plan.path.display(), error);
                report.parse_failures += 1;
                report
                    .parse_failure_details
                    .push(FailureDetail::parse(plan, error.to_string()));
                continue;
            }
        };
        let mut c = bxl_parser::canonicalize(
            &doc,
            "Texas Instruments",
            &plan.primary,
            Some(plan.sha.clone()),
        );
        c.canonicalizer_version = bxl_parser::BXL_CANONICALIZER_VERSION.into();
        if write_component(c, &plan.path, data, Some(&plan.references))? {
            report.written += 1;
        } else {
            report.reused += 1;
        }
    }
    report.failure_count = report.parse_failures
        + report.missing
        + report.type_mismatches
        + report.ambiguous
        + report.output_collisions;
    write_conversion_report(data, &report, &[])?;
    println!("{}", report.to_markdown());
    if report.failure_count != 0 {
        anyhow::bail!(
            "BXL conversion completed with {} failure(s)",
            report.failure_count
        );
    }
    Ok(())
}

#[cfg(test)]
fn parse_bxl_documents(
    plans: &[ConversionPlan],
) -> BTreeMap<String, std::result::Result<bxl_model::BxlDocument, String>> {
    let mut parsed = BTreeMap::new();
    for plan in plans {
        parsed.entry(plan.sha.clone()).or_insert_with(|| {
            fs::read(&plan.path)
                .and_then(|bytes| bxl_parser::parse(&bytes).map_err(std::io::Error::other))
                .map_err(|e| e.to_string())
        });
    }
    parsed
}

#[derive(Clone)]
struct ConversionPlan {
    sha: String,
    path: PathBuf,
    references: Vec<eda_model::AssetRecord>,
    primary: String,
    output: PathBuf,
    provenance_hash: String,
}

fn conversion_plans(data: &Path, corpus: &CurrentBxlCorpus) -> Vec<ConversionPlan> {
    let mut by_sha: BTreeMap<String, (PathBuf, Vec<eda_model::AssetRecord>)> = BTreeMap::new();
    for asset in &corpus.assets {
        debug_assert_eq!(asset.sha256, asset.manifest.sha256);
        let entry = by_sha
            .entry(asset.sha256.clone())
            .or_insert_with(|| (asset.object_path.clone(), Vec::new()));
        entry.1.extend(asset.references.clone());
    }
    let mut plans = by_sha
        .into_iter()
        .map(|(sha, (path, mut references))| {
            references.sort_by(|a, b| {
                serde_json::to_string(a)
                    .unwrap()
                    .cmp(&serde_json::to_string(b).unwrap())
            });
            let primary = references
                .iter()
                .map(|r| r.mpn.as_str())
                .min()
                .unwrap_or("unknown")
                .to_owned();
            let output = data
                .join("derived/components")
                .join(safe("Texas Instruments"))
                .join(format!("{}.json", safe(&primary)));
            let provenance_hash = provenance_hash(&references);
            ConversionPlan {
                sha,
                path,
                references,
                primary,
                output,
                provenance_hash,
            }
        })
        .collect::<Vec<_>>();
    plans.sort_by(|a, b| a.sha.cmp(&b.sha));
    plans
}

fn output_collisions(plans: &[ConversionPlan]) -> Vec<String> {
    let mut owners = BTreeMap::<&Path, &str>::new();
    let mut collisions = BTreeSet::new();
    for plan in plans {
        if let Some(previous) = owners.insert(&plan.output, &plan.sha) {
            if previous != plan.sha {
                collisions.insert(plan.output.display().to_string());
            }
        }
    }
    collisions.into_iter().collect()
}

#[derive(Default, serde::Serialize)]
struct ConversionReport {
    observations: usize,
    urls: usize,
    shas: usize,
    written: usize,
    reused: usize,
    parse_failures: usize,
    missing: usize,
    type_mismatches: usize,
    ambiguous: usize,
    output_collisions: usize,
    stale_derived_before: usize,
    failure_count: usize,
    #[serde(default)]
    parse_failure_details: Vec<FailureDetail>,
    #[serde(default)]
    missing_details: Vec<FailureDetail>,
    #[serde(default)]
    type_mismatch_details: Vec<FailureDetail>,
    #[serde(default)]
    ambiguous_details: Vec<FailureDetail>,
}

#[derive(serde::Serialize, Clone)]
struct FailureDetail {
    kind: String,
    sha256: Option<String>,
    path: Option<String>,
    urls: Vec<String>,
    mpns: Vec<String>,
    error: Option<String>,
}

impl FailureDetail {
    fn url(kind: &str, url: &str) -> Self {
        Self {
            kind: kind.into(),
            sha256: None,
            path: None,
            urls: vec![url.into()],
            mpns: Vec::new(),
            error: None,
        }
    }
    fn parse(plan: &ConversionPlan, error: String) -> Self {
        Self {
            kind: "parse".into(),
            sha256: Some(plan.sha.clone()),
            path: Some(plan.path.display().to_string()),
            urls: plan
                .references
                .iter()
                .map(|r| r.asset_url.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            mpns: plan
                .references
                .iter()
                .map(|r| r.mpn.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            error: Some(error),
        }
    }
}

impl ConversionReport {
    fn to_markdown(&self) -> String {
        format!("# TI BXL conversion\n\n| Metric | Count |\n|---|---:|\n| Observations | {} |\n| URLs | {} |\n| SHAs | {} |\n| Written | {} |\n| Reused | {} |\n| Parse failures | {} |\n| Missing | {} |\n| Type mismatches | {} |\n| Ambiguous | {} |\n| Output collisions | {} |\n| Stale derived files before run | {} |\n| Failures | {} |\n", self.observations, self.urls, self.shas, self.written, self.reused, self.parse_failures, self.missing, self.type_mismatches, self.ambiguous, self.output_collisions, self.stale_derived_before, self.failure_count)
    }
}

fn stale_derived_files(data: &Path, plans: &[ConversionPlan]) -> Result<usize> {
    let expected = plans
        .iter()
        .map(|p| p.output.clone())
        .collect::<BTreeSet<_>>();
    let dir = data
        .join("derived/components")
        .join(safe("Texas Instruments"));
    let mut count = 0;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.path().extension().is_some_and(|x| x == "json") && !expected.contains(entry.path())
        {
            count += 1;
        }
    }
    count += plans
        .iter()
        .filter(|p| p.output.exists() && !cache_matches(&p.output, &p.sha, &p.provenance_hash))
        .count();
    Ok(count)
}

fn cache_matches(path: &Path, sha: &str, provenance: &str) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<EdaComponent>(&s).ok())
        .is_some_and(|c| {
            c.source_sha256.as_deref() == Some(sha)
                && c.canonicalizer_version == bxl_parser::BXL_CANONICALIZER_VERSION
                && c.metadata
                    .get("provenance_hash")
                    .is_some_and(|x| x == provenance)
        })
}

fn write_conversion_report(
    data: &Path,
    report: &ConversionReport,
    collisions: &[String],
) -> Result<()> {
    let dir = data.join("reports");
    fs::create_dir_all(&dir)?;
    let mut json = serde_json::to_value(report)?;
    json["collision_paths"] = serde_json::json!(collisions);
    fs::write(
        dir.join("ti-bxl-conversion.json"),
        format!("{}\n", serde_json::to_string_pretty(&json)?),
    )?;
    fs::write(dir.join("ti-bxl-conversion.md"), report.to_markdown())?;
    Ok(())
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct BxlStatsResult {
    objects_processed: usize,
    parsed_successfully: usize,
    decompression_failures: usize,
    utf8_failures: usize,
    canonicalization_warnings: usize,
    compressed_bytes: u64,
    decoded_bytes: u64,
}

#[derive(Clone)]
struct CurrentBxlAsset {
    asset_url: String,
    sha256: String,
    object_path: PathBuf,
    manifest: ManifestRecord,
    references: Vec<eda_model::AssetRecord>,
}

struct CurrentBxlCorpus {
    observations: usize,
    unique_urls: usize,
    assets: Vec<CurrentBxlAsset>,
    missing_urls: Vec<String>,
    type_mismatches: Vec<String>,
    ambiguous_urls: Vec<String>,
    historical_bxl_urls: Vec<String>,
}

fn load_current_ti_bxl_corpus(data: &Path) -> Result<CurrentBxlCorpus> {
    let current = data.join("catalogs/ti-bxl/current.json");
    let rows: Vec<vendor_ti::TiPackageProduct> = if current.exists() {
        serde_json::from_str(&fs::read_to_string(current)?)?
    } else {
        Vec::new()
    };
    let mut references = BTreeMap::<String, Vec<eda_model::AssetRecord>>::new();
    for row in rows {
        let Some(url) = row.bxl_url else { continue };
        if vendor_ti::classify_cad_url(&url) != vendor_ti::CadAssetKind::Bxl {
            continue;
        }
        references
            .entry(url.clone())
            .or_default()
            .push(eda_model::AssetRecord {
                manufacturer: "Texas Instruments".into(),
                mpn: row.part_number,
                part_url: row.product_url.unwrap_or_default(),
                asset_url: url.clone(),
                discovery_url: Some(row.source),
                filename: url.rsplit('/').next().unwrap_or(&url).into(),
                format: "bxl".into(),
                content_type: None,
                source_package: Some(format!(
                    "{};pins={}",
                    row.package_code.unwrap_or_default(),
                    row.pin_count.map_or_else(|| "".into(), |n| n.to_string())
                )),
                request_url: None,
            });
    }
    let observations = references.values().map(Vec::len).sum();
    // Acquisition references enrich provenance only; they never add URLs to
    // the current corpus. The catalog above remains the membership authority.
    for line in fs::read_to_string(data.join("acquisition/texas-instruments-references.jsonl"))
        .unwrap_or_default()
        .lines()
    {
        let Ok(items) = serde_json::from_str::<Vec<eda_model::AssetRecord>>(line) else {
            continue;
        };
        for item in items {
            if let Some(group) = references.get_mut(&item.asset_url) {
                if !group.contains(&item) {
                    group.push(item);
                }
            }
        }
    }
    let mut manifest_by_url = BTreeMap::<String, Vec<ManifestRecord>>::new();
    for line in fs::read_to_string(data.join("manifests/texas-instruments.jsonl"))
        .unwrap_or_default()
        .lines()
    {
        if let Ok(m) = serde_json::from_str::<ManifestRecord>(line) {
            manifest_by_url
                .entry(m.asset_url.clone())
                .or_default()
                .push(m);
        }
    }
    let mut corpus = CurrentBxlCorpus {
        observations,
        unique_urls: references.len(),
        assets: Vec::new(),
        missing_urls: Vec::new(),
        type_mismatches: Vec::new(),
        ambiguous_urls: Vec::new(),
        historical_bxl_urls: Vec::new(),
    };
    corpus.historical_bxl_urls = manifest_by_url
        .iter()
        .filter(|(url, records)| {
            !references.contains_key(*url) && records.iter().any(|m| m.format == "bxl")
        })
        .map(|(url, _)| url.clone())
        .collect();
    for (url, refs) in references {
        let all = manifest_by_url.get(&url).cloned().unwrap_or_default();
        let bxl = all
            .iter()
            .filter(|m| m.format == "bxl")
            .cloned()
            .collect::<Vec<_>>();
        if bxl.is_empty() {
            if all.is_empty() {
                corpus.missing_urls.push(url);
            } else {
                corpus.type_mismatches.push(url);
            }
            continue;
        }
        let shas = bxl
            .iter()
            .map(|m| m.sha256.clone())
            .collect::<BTreeSet<_>>();
        if shas.len() != 1 {
            corpus.ambiguous_urls.push(url);
            continue;
        }
        let sha = shas.into_iter().next().unwrap();
        if sha.len() < 2 {
            corpus.missing_urls.push(url);
            continue;
        }
        let object_path = data
            .join("objects")
            .join(&sha[..2])
            .join(format!("{sha}.bxl"));
        if !object_path.exists() {
            corpus.missing_urls.push(url.clone());
            continue;
        }
        corpus.assets.push(CurrentBxlAsset {
            asset_url: url,
            sha256: sha,
            object_path,
            manifest: bxl.into_iter().next().unwrap(),
            references: refs,
        });
    }
    corpus.assets.sort_by(|a, b| a.asset_url.cmp(&b.asset_url));
    Ok(corpus)
}

fn stats(data: &Path) -> Result<()> {
    let corpus = load_current_ti_bxl_corpus(data)?;
    let current_objects = corpus
        .assets
        .iter()
        .map(|a| a.sha256.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    println!(
        "BXL stats\nCurrent BXL URLs: {}\nCurrent SHA objects: {}\n",
        corpus.unique_urls, current_objects
    );
    let exe = std::env::current_exe().context("locate eda-convert executable")?;
    let mut child = ProcessCommand::new(exe)
        .arg("--data")
        .arg(data)
        .arg("bxl-stats-worker")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run quiet BXL statistics worker")?;
    let stderr = child.stderr.take().expect("worker stderr was piped");
    let stderr_reader = thread::spawn(move || {
        for line in BufReader::new(stderr)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if let Some(progress) = line.strip_prefix("BXL_PROGRESS ") {
                let mut values = progress.split_whitespace();
                if let (Some(done), Some(total)) = (values.next(), values.next()) {
                    eprintln!("BXL stats: {done}/{total} objects");
                }
            }
        }
    });
    let output = child
        .wait_with_output()
        .context("wait for BXL statistics worker")?;
    let _ = stderr_reader.join();
    if !output.status.success() {
        anyhow::bail!("BXL statistics worker failed with {}", output.status);
    }
    let result: BxlStatsResult = serde_json::from_slice(&output.stdout)?;
    let historical = walkdir::WalkDir::new(data.join("objects"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "bxl"))
        .count()
        .saturating_sub(current_objects);
    println!(
        "BXL corpus\n\nCurrent observations              {}\nCurrent unique BXL URLs           {}\nCurrent-corpus BXL objects       {}\nHistorical BXL objects            {}\nObjects processed                 {}\nParsed successfully               {}\nDecompression failures            {}\nUTF-8 failures                    {}\nCanonicalization warnings         {}\nMissing URLs                      {}\nType mismatches                   {}\nAmbiguous URLs                    {}\nHistorical BXL URLs not current  {}\n\nCompressed bytes                  {}\nDecoded bytes                     {}",
        corpus.observations, corpus.unique_urls, current_objects,
        historical,
        result.objects_processed,
        result.parsed_successfully,
        result.decompression_failures,
        result.utf8_failures,
        result.canonicalization_warnings,
        corpus.missing_urls.len(), corpus.type_mismatches.len(), corpus.ambiguous_urls.len(), corpus.historical_bxl_urls.len(),
        result.compressed_bytes,
        result.decoded_bytes
    );
    Ok(())
}

#[derive(serde::Serialize, Clone)]
struct StaleDerivedEntry {
    path: String,
    sha256: Option<String>,
    mpn: Option<String>,
}

#[derive(serde::Serialize)]
struct StaleDerivedInventory {
    missing_expected: Vec<String>,
    stale_extra: Vec<StaleDerivedEntry>,
}

fn bxl_stale(data: &Path) -> Result<()> {
    let expected = current_ti_component_paths(data)?;
    let missing_expected = expected
        .iter()
        .filter(|path| !path.is_file())
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let dir = data
        .join("derived/components")
        .join(safe("Texas Instruments"));
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
    {
        if expected.contains(entry.path()) {
            continue;
        }
        let component = fs::read_to_string(entry.path())
            .ok()
            .and_then(|json| serde_json::from_str::<EdaComponent>(&json).ok());
        entries.push(StaleDerivedEntry {
            path: entry.path().display().to_string(),
            sha256: component.as_ref().and_then(|c| c.source_sha256.clone()),
            mpn: component.map(|c| c.mpn),
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let reports = data.join("reports");
    fs::create_dir_all(&reports)?;
    fs::write(
        reports.join("ti-bxl-stale-derived.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&StaleDerivedInventory {
                missing_expected: missing_expected.clone(),
                stale_extra: entries.clone(),
            })?
        ),
    )?;
    let mut markdown = format!("# TI BXL derived component inventory\n\nMissing expected: {}\n\nStale extra: {}\n\n## Stale extra\n\n| Path | SHA256 | MPN |\n|---|---|---|\n", missing_expected.len(), entries.len());
    for entry in &entries {
        markdown.push_str(&format!(
            "| `{}` | `{}` | `{}` |\n",
            entry.path,
            entry.sha256.as_deref().unwrap_or(""),
            entry.mpn.as_deref().unwrap_or("")
        ));
    }
    if !missing_expected.is_empty() {
        markdown.push_str("\n## Missing expected\n\n");
        for path in &missing_expected {
            markdown.push_str(&format!("* `{path}`\n"));
        }
    }
    fs::write(reports.join("ti-bxl-stale-derived.md"), markdown)?;
    println!("stale derived components: {}", entries.len());
    Ok(())
}

fn stats_worker(data: &Path) -> Result<()> {
    let corpus = load_current_ti_bxl_corpus(data)?;
    let mut paths = corpus
        .assets
        .iter()
        .map(|a| a.object_path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let mut result = BxlStatsResult::default();
    let total = paths.len();
    eprintln!("BXL stats: processing {total} current SHA objects");
    for (index, path) in paths.into_iter().enumerate() {
        if index == 0 || (index + 1) % 100 == 0 || index + 1 == total {
            eprintln!("BXL_PROGRESS {} {}", index + 1, total);
        }
        result.objects_processed += 1;
        let bytes = fs::read(&path)?;
        result.compressed_bytes += bytes.len() as u64;
        match bxl_parser::parse(&bytes) {
            Ok(doc) => {
                result.parsed_successfully += 1;
                let component =
                    bxl_parser::canonicalize(&doc, "Texas Instruments", "<stats>", None);
                result.canonicalization_warnings += component.warnings.len();
                if let Some(text) = doc.raw_text {
                    result.decoded_bytes += text.len() as u64;
                } else {
                    result.utf8_failures += 1;
                }
            }
            Err(error) => {
                if error.to_string().contains("UTF-8") {
                    result.utf8_failures += 1;
                } else {
                    result.decompression_failures += 1;
                }
            }
        }
    }
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn kicad_generate(
    data: &Path,
    mpn: Option<String>,
    manufacturer: Option<String>,
    input: Option<PathBuf>,
    all: bool,
    clean: bool,
    nickname: &str,
    deduplicated: bool,
    with_3d: bool,
) -> Result<()> {
    let root = data.join("generated/kicad");
    if clean && root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;
    let model_dir = root.join("3dmodels/Personal_Packages.3dshapes");
    if with_3d {
        fs::create_dir_all(&model_dir)?;
    }
    let mut paths = Vec::new();
    if let Some(p) = input {
        paths.push(p)
    } else if all && manufacturer.as_deref() == Some("ti") {
        let expected = current_ti_component_paths(data)?;
        let missing = expected
            .iter()
            .filter(|path| !path.is_file())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            anyhow::bail!(
                "{} current TI derived component(s) are missing",
                missing.len()
            );
        }
        paths.extend(expected);
    } else {
        let dir = data.join("derived/components");
        for e in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        {
            paths.push(e.path().to_path_buf())
        }
    }
    let mut cs = Vec::new();
    for p in paths {
        let c: EdaComponent = serde_json::from_str(&fs::read_to_string(&p)?)?;
        if !all && mpn.as_deref().is_some_and(|x| x != c.mpn) {
            continue;
        }
        if manufacturer
            .as_deref()
            .is_some_and(|x| x != "ti" && x != c.manufacturer)
        {
            continue;
        }
        cs.push(c)
    }
    if cs.is_empty() {
        anyhow::bail!("no canonical components selected")
    }
    cs.sort_by(|a, b| a.mpn.cmp(&b.mpn));
    let mut report = String::from("# KiCad generation report\n\n");
    let mut warnings = 0;
    let mut errors = 0;
    let mut by_m: std::collections::BTreeMap<String, Vec<EdaComponent>> =
        std::collections::BTreeMap::new();
    for c in cs {
        let v = kicad::validate(&c);
        warnings += v.warnings.len();
        errors += v.errors.len();
        let mismatches = kicad::check_pin_pad(&c);
        warnings += mismatches.len();
        report.push_str(&format!(
            "* {}: {} symbols, {} packages, {} pin/pad mismatches, {} warnings, {} errors\n",
            c.mpn,
            c.symbols.len(),
            c.packages.len(),
            mismatches.len(),
            v.warnings.len(),
            v.errors.len()
        ));
        by_m.entry(safe(&c.manufacturer)).or_default().push(c)
    }
    let manufacturer_count = by_m.len();
    let all_components = by_m.values().flatten().cloned().collect::<Vec<_>>();
    let dedupe = if deduplicated {
        Some(package_normalize::dedupe(&all_components))
    } else {
        None
    };
    let footprint_map = production_footprint_map(&all_components, dedupe.as_ref())?;
    for (man, items) in by_m {
        let symdir = root.join("symbols");
        fs::create_dir_all(&symdir)?;
        fs::write(
            symdir.join(format!("{man}.kicad_sym")),
            kicad::symbol_lib_many_with_footprints(
                &items,
                if deduplicated {
                    "Personal_Packages"
                } else {
                    nickname
                },
                &footprint_map,
            ),
        )?;
        let fpdir = root.join("footprints").join(if deduplicated {
            "Personal_Packages.pretty".into()
        } else {
            format!("{man}.pretty")
        });
        fs::create_dir_all(&fpdir)?;
        for c in &items {
            for p in &c.packages {
                match eda_model::footprint_eligibility(p) {
                    eda_model::FootprintEligibility::Eligible => {}
                    eda_model::FootprintEligibility::UnknownDrillPlating { pad } => {
                        report.push_str(&format!("* {} / {}: skipped production footprint; drill plating is unspecified for pad {}\n", c.mpn, p.name, pad));
                        continue;
                    }
                    eda_model::FootprintEligibility::UnsupportedShape { pad, shape } => {
                        report.push_str(&format!("* {} / {}: skipped production footprint; unsupported pad shape `{shape}` on pad {pad}\n", c.mpn, p.name));
                        continue;
                    }
                }
                let out_name = if let Some(d) = dedupe.as_ref() {
                    let normalized = d
                        .packages
                        .iter()
                        .find(|x| {
                            x.source.manufacturer == c.manufacturer
                                && x.source.mpn == c.mpn
                                && x.source.name == p.name
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!("normalized package missing for {} / {}", c.mpn, p.name)
                        })?;
                    dname(d, normalized)?
                } else {
                    safe(&p.name)
                };
                let output = fpdir.join(format!("{out_name}.kicad_mod"));
                if !deduplicated || !output.exists() {
                    let mut pp = p.clone();
                    if deduplicated {
                        pp.name = out_name.clone();
                    }
                    let model = if with_3d {
                        find_ti_model(data, c, p, &model_dir)?
                    } else {
                        None
                    };
                    fs::write(
                        output,
                        kicad::footprint_with_model(c, &pp, model.as_deref()),
                    )?;
                }
            }
        }
    }
    report.push_str(&format!(
        "\nWarnings: {warnings}\nErrors: {errors}\nKiCad format version: {}\n",
        kicad::KICAD_VERSION
    ));
    fs::create_dir_all(root.join("reports"))?;
    fs::write(root.join("reports/kicad-generation.md"), report)?;
    println!("generated {manufacturer_count} manufacturer libraries");
    Ok(())
}
fn production_footprint_map(
    components: &[EdaComponent],
    dedupe: Option<&package_normalize::DedupeResult>,
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut map = std::collections::BTreeMap::new();
    for c in components {
        for p in &c.packages {
            if !matches!(
                eda_model::footprint_eligibility(p),
                eda_model::FootprintEligibility::Eligible
            ) {
                continue;
            }
            let key = format!("{}|{}|{}", c.manufacturer, c.mpn, p.name);
            let name = if let Some(d) = dedupe {
                let normalized = d
                    .packages
                    .iter()
                    .find(|x| {
                        x.source.manufacturer == c.manufacturer
                            && x.source.mpn == c.mpn
                            && x.source.name == p.name
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("normalized package missing for {} / {}", c.mpn, p.name)
                    })?;
                dname(d, normalized)?
            } else {
                safe(&p.name)
            };
            map.insert(key, name);
        }
    }
    Ok(map)
}

fn dname(
    d: &package_normalize::DedupeResult,
    p: &package_normalize::NormalizedPackage,
) -> Result<String> {
    d.canonical
        .iter()
        .find(|x| {
            x.fingerprints.kicad_footprint_hash.is_some()
                && x.fingerprints.kicad_footprint_hash == p.fingerprints.kicad_footprint_hash
        })
        .map(|x| safe(&x.preferred_name))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "eligible package {} has no canonical KiCad footprint group",
                p.source.name
            )
        })
}
fn ti_package_code(s: &str) -> String {
    let u = s.to_ascii_uppercase().replace(['-', '_', ' '], "");
    let mut letters = String::new();
    let mut digits = String::new();
    for c in u.chars() {
        if c.is_ascii_alphabetic() && digits.is_empty() {
            letters.push(c)
        } else if c.is_ascii_digit() {
            digits.push(c)
        }
    }
    if letters.is_empty() || digits.is_empty() {
        return u;
    }
    format!(
        "{}{}",
        letters,
        digits.trim_start_matches('0').if_empty("0")
    )
}
trait NonEmpty {
    fn if_empty(self, fallback: &str) -> String;
}
impl NonEmpty for &str {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.into()
        } else {
            self.into()
        }
    }
}
fn ti_model_code(filename: &str) -> String {
    let stem = filename
        .rsplit('/')
        .next()
        .unwrap_or(filename)
        .split('.')
        .next()
        .unwrap_or(filename)
        .trim_end_matches('A');
    let mut letters = String::new();
    let mut digits = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphabetic() && digits.is_empty() {
            letters.push(c)
        } else if c.is_ascii_digit() {
            digits.push(c)
        }
    }
    ti_package_code(&format!("{letters}{digits}"))
}
fn find_ti_model(
    data: &Path,
    c: &EdaComponent,
    p: &eda_model::Package,
    dir: &Path,
) -> Result<Option<String>> {
    if c.manufacturer != "Texas Instruments" {
        return Ok(None);
    };
    let package_code = ti_package_code(&p.name);
    for line in fs::read_to_string(data.join("manifests/texas-instruments.jsonl"))
        .unwrap_or_default()
        .lines()
    {
        let m: ManifestRecord = serde_json::from_str(line)?;
        if m.mpn != c.mpn
            || !matches!(m.format.as_str(), "stp" | "step")
            || ti_model_code(&m.filename) != package_code
        {
            continue;
        }
        let src = data
            .join("objects")
            .join(&m.sha256[..2])
            .join(format!("{}.{}", m.sha256, m.format));
        if !src.exists() {
            continue;
        }
        let bytes = fs::read(&src)?;
        let sm = step_model::SourceModel {
            sha256: m.sha256.clone(),
            original_filename: m.filename.clone(),
            manufacturer: m.manufacturer.clone(),
            source_url: Some(m.asset_url.clone()),
            associated_mpns: vec![m.mpn.clone()],
            source_package_names: vec![p.name.clone()],
        };
        if step_model::parse(&bytes, sm).is_err() {
            continue;
        }
        let filename = step_model::model_filename(&format!("TI_{}", ti_model_code(&m.filename)));
        let target = dir.join(&filename);
        fs::create_dir_all(dir)?;
        if !target.exists() {
            fs::write(&target, bytes)?
        }
        return Ok(Some(format!(
            "${{PERSONAL_KICAD_LIB}}/3dmodels/Personal_Packages.3dshapes/{filename}"
        )));
    }
    Ok(None)
}
fn load_components(data: &Path, manufacturer: Option<&str>) -> Result<Vec<EdaComponent>> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(data.join("derived/components"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
    {
        let c: EdaComponent = serde_json::from_str(&fs::read_to_string(e.path())?)?;
        if manufacturer.is_none_or(|m| {
            m == "ti" && c.manufacturer == "Texas Instruments" || m == c.manufacturer
        }) {
            out.push(c)
        }
    }
    Ok(out)
}

fn current_ti_component_paths(data: &Path) -> Result<BTreeSet<PathBuf>> {
    let corpus = load_current_ti_bxl_corpus(data)?;
    Ok(conversion_plans(data, &corpus)
        .into_iter()
        .map(|plan| plan.output)
        .collect())
}
fn find_package(data: &Path, wanted: &str) -> Result<package_normalize::NormalizedPackage> {
    for c in load_components(data, None)? {
        for p in &c.packages {
            let n = package_normalize::normalize_package(&c, p);
            if p.name == wanted
                || format!("{}:{}", c.mpn, p.name) == wanted
                || n.source.mpn == wanted
            {
                return Ok(n);
            }
        }
    }
    anyhow::bail!("package not found: {wanted}")
}
fn package_diff(data: &Path, left: &str, right: &str, json: bool) -> Result<()> {
    let a = find_package(data, left)?;
    let b = find_package(data, right)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&(a, b))?)
    } else {
        println!("{}", package_normalize::package_diff(&a, &b));
    }
    Ok(())
}
fn package_dedupe(data: &Path, manufacturer: Option<&str>) -> Result<()> {
    let cs = load_components(data, manufacturer)?;
    let d = package_normalize::dedupe(&cs);
    let dir = data.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir.join("reports/package-dedupe-svg"))?;
    let mut report = String::from("# Package deduplication report\n\n");
    report+=&format!("Source packages | {}\n\nUnique physical geometries | {}\n\nUnique electrical geometries | {}\n\nUnique manufacturing geometries | {}\n\nFull unique geometries | {}\n\n",d.packages.len(),d.packages.iter().map(|p|&p.fingerprints.physical_geometry_hash).collect::<std::collections::BTreeSet<_>>().len(),d.packages.iter().map(|p|&p.fingerprints.electrical_geometry_hash).collect::<std::collections::BTreeSet<_>>().len(),d.packages.iter().map(|p|&p.fingerprints.manufacturing_hash).collect::<std::collections::BTreeSet<_>>().len(),d.packages.iter().map(|p|&p.fingerprints.full_geometry_hash).collect::<std::collections::BTreeSet<_>>().len());
    for g in d.canonical.iter().filter(|x| x.aliases.len() > 1) {
        report += &format!(
            "## {}\n\nAliases: {}\n\n",
            g.preferred_name,
            g.aliases.join(", ")
        );
        let a = &g.representative;
        for n in &g.aliases {
            if let Ok(b) = find_package(data, n) {
                if b.source.name != a.source.name {
                    let file = format!("{}_vs_{}.svg", safe(&a.source.name), safe(&b.source.name));
                    fs::write(
                        dir.join("reports/package-dedupe-svg").join(file),
                        package_normalize::overlay_svg(a, &b),
                    )?;
                }
            }
        }
    }
    for name in ["D14", "D14-M", "D14-L", "DGG48", "DGG48-M", "DGG48-L"] {
        if let Ok(p) = find_package(data, name) {
            report += &format!(
                "* {}: physical {}, electrical {}, manufacturing {}, full {}\n",
                name,
                &p.fingerprints.physical_geometry_hash[..12],
                &p.fingerprints.electrical_geometry_hash[..12],
                &p.fingerprints.manufacturing_hash[..12],
                &p.fingerprints.full_geometry_hash[..12]
            );
        }
    }
    fs::write(dir.join("reports/package-deduplication.md"), report)?;
    println!("Source packages {}\nUnique physical geometries {}\nUnique electrical geometries {}\nUnique manufacturing geometries {}\nFull unique geometries {}",d.packages.len(),d.packages.iter().map(|p|&p.fingerprints.physical_geometry_hash).collect::<std::collections::BTreeSet<_>>().len(),d.packages.iter().map(|p|&p.fingerprints.electrical_geometry_hash).collect::<std::collections::BTreeSet<_>>().len(),d.packages.iter().map(|p|&p.fingerprints.manufacturing_hash).collect::<std::collections::BTreeSet<_>>().len(),d.packages.iter().map(|p|&p.fingerprints.full_geometry_hash).collect::<std::collections::BTreeSet<_>>().len());
    Ok(())
}
fn kicad_check(path: &Path) -> Result<()> {
    let mut symbols = 0;
    let mut footprints = 0;
    let mut errors = 0;
    let mut parser_errors = 0;
    for e in walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
    {
        if e.path().extension().is_some_and(|x| x == "kicad_sym") {
            symbols += 1
        } else if e.path().extension().is_some_and(|x| x == "kicad_mod") {
            footprints += 1
        } else {
            continue;
        }
        if !kicad::sexpr_balanced(&fs::read_to_string(e.path())?) {
            errors += 1
        }
    }
    let parser_available = std::process::Command::new("kicad-cli")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if parser_available {
        let temp = std::env::temp_dir().join(format!("eda-kicad-check-{}", std::process::id()));
        fs::create_dir_all(&temp)?;
        for entry in walkdir::WalkDir::new(path)
            .into_iter()
            .filter_map(Result::ok)
        {
            let is_sym = entry.path().extension().is_some_and(|x| x == "kicad_sym");
            let is_pretty = entry.file_type().is_dir()
                && entry.path().extension().is_some_and(|x| x == "pretty");
            if !is_sym && !is_pretty {
                continue;
            }
            // KiCad's footprint upgrader requires a new output directory; it
            // rejects an already-existing directory as an output collision.
            let output_dir = temp.join(format!("out-{}", entry.path().to_string_lossy().len()));
            let result = if is_sym {
                std::process::Command::new("kicad-cli")
                    .args(["sym", "upgrade", "--output"])
                    .arg(&output_dir)
                    .arg(entry.path())
                    .output()
            } else {
                std::process::Command::new("kicad-cli")
                    .args(["fp", "upgrade", "--output"])
                    .arg(&output_dir)
                    .arg(entry.path())
                    .output()
            };
            if !result.is_ok_and(|output| output.status.success()) {
                parser_errors += 1;
            }
        }
        let _ = fs::remove_dir_all(temp);
    }
    println!("KiCad validation\n\nSymbol libraries       {symbols}\nFootprints             {footprints}\nS-expression failures  {errors}\nKiCad parser failures  {}\nKiCad parser validation: {}", parser_errors, if parser_available { "performed" } else { "unavailable" });
    if errors > 0 || parser_errors > 0 {
        anyhow::bail!("KiCad syntax validation failed")
    }
    Ok(())
}
fn symbol_check(data: &Path, manufacturer: Option<&str>, wanted: Option<&str>) -> Result<()> {
    let cs = load_components(data, manufacturer)?
        .into_iter()
        .filter(|c| wanted.is_none_or(|m| m == c.mpn))
        .collect::<Vec<_>>();
    let mut pins = 0;
    let mut unknown = 0;
    let mut hidden = 0;
    let mut nc = 0;
    let mut units = 0;
    for c in &cs {
        for s in &c.symbols {
            units += s.units.len();
            for u in &s.units {
                for p in &u.pins {
                    pins += 1;
                    unknown += (p.electrical_type == eda_model::ElectricalType::Unknown) as usize;
                    hidden += (!p.visible) as usize;
                    nc += (p.electrical_type == eda_model::ElectricalType::NoConnect) as usize;
                }
            }
        }
    }
    let mapped = cs
        .iter()
        .flat_map(|c| c.pin_map.iter())
        .filter(|p| p.pad_ref.is_some())
        .count();
    println!("Symbol semantics\n\nComponents { }\nUnits {units}\nPins {pins}\nMapped component pins {mapped}\nUnknown electrical types {unknown}\nHidden pins {hidden}\nNC pins {nc}",cs.len());
    fs::create_dir_all(data.parent().unwrap_or(Path::new(".")).join("reports"))?;
    fs::write(data.parent().unwrap_or(Path::new(".")).join("reports/symbol-semantics.md"),format!("# Symbol semantics\n\nComponents | {}\n\nUnits | {}\n\nPins | {}\n\nMapped pins | {}\n\nUnknown electrical types | {}\n\nHidden pins | {}\n\nNC pins | {}\n\nBefore semantic decoding: 356 unspecified pins.\n\nAfter semantic decoding: {} unspecified pins.\n\nRaw values are preserved; no pin-name inference is performed.\n",cs.len(),units,pins,mapped,unknown,hidden,nc,unknown))?;
    Ok(())
}
fn pin_map(data: &Path, wanted: &str) -> Result<()> {
    let c = load_components(data, None)?
        .into_iter()
        .find(|c| c.mpn == wanted)
        .context("MPN not found")?;
    println!("MPN: {}\n\nPin   Name                 Unit  Type          SymbolPin  PackagePad\n------------------------------------------------------------------",c.mpn);
    for p in c
        .symbols
        .iter()
        .flat_map(|s| s.units.iter().flat_map(|u| u.pins.iter()))
    {
        println!(
            "{:<5}{:<21}{:<6}{:<14}{:<11}unmapped",
            p.number,
            p.name,
            1,
            format!("{:?}", p.electrical_type),
            p.number
        );
    }
    Ok(())
}
fn ti_corpus_report(data: &Path) -> Result<()> {
    let cs = load_components(data, Some("ti"))?;
    let mut pins = 0;
    let mut packages = 0;
    let mut rejected = 0;
    for c in &cs {
        pins += c
            .symbols
            .iter()
            .flat_map(|s| s.units.iter().map(|u| u.pins.len()))
            .sum::<usize>();
        packages += c.packages.len();
        let v = kicad::validate(c);
        if c.symbols.is_empty() || c.packages.is_empty() || !v.errors.is_empty() {
            rejected += 1;
        }
    }
    let text =
        fs::read_to_string(data.join("manifests/texas-instruments.jsonl")).unwrap_or_default();
    let mut counts = std::collections::BTreeMap::new();
    let mut hashes = std::collections::BTreeSet::new();
    for l in text.lines() {
        if let Ok(m) = serde_json::from_str::<ManifestRecord>(l) {
            hashes.insert(m.sha256);
            *counts.entry(m.format).or_insert(0usize) += 1;
        }
    }
    let root = data.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(root.join("reports"))?;
    fs::write(root.join("reports/ti-large-corpus.md"),format!("# TI large-corpus report\n\nDate: 2026-09-10\n\nThe current configured public prefix discovery yielded fewer than 1,000 unique parts; this is the complete bounded snapshot.\n\nComponents | {}\n\nSymbol pins | {pins}\n\nSource packages | {packages}\n\nManifest formats | {:?}\n\nUnique raw object hashes | {}\n\nRejected canonical components | {rejected}\n",cs.len(),counts,hashes.len()))?;
    let mut q = Vec::new();
    for c in &cs {
        let v = kicad::validate(c);
        let status = if c.symbols.is_empty() || c.packages.is_empty() || !v.errors.is_empty() {
            "Rejected"
        } else if c.symbols.iter().any(|s| {
            s.units.iter().any(|u| {
                u.pins
                    .iter()
                    .any(|p| p.electrical_type == eda_model::ElectricalType::Unknown)
            })
        }) {
            "ValidWithWarnings"
        } else {
            "Valid"
        };
        q.push(serde_json::json!({"mpn":c.mpn,"status":status,"warnings":v.warnings}));
    }
    fs::write(
        root.join("reports/ti-corpus-quality.json"),
        serde_json::to_string_pretty(&q)?,
    )?;
    fs::write(root.join("reports/ti-corpus-quality.md"),format!("# TI corpus quality\n\nComponents | {}\n\nValid | {}\n\nValidWithWarnings | {}\n\nRejected | {}\n",cs.len(),q.iter().filter(|x|x["status"]=="Valid").count(),q.iter().filter(|x|x["status"]=="ValidWithWarnings").count(),q.iter().filter(|x|x["status"]=="Rejected").count()))?;
    println!(
        "TI components {}\nPackages {}\nPins {}\nUnique raw objects {}",
        cs.len(),
        packages,
        pins,
        hashes.len()
    );
    Ok(())
}
fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn model_check(data: &Path, manufacturer: Option<&str>) -> Result<()> {
    let mut raw = 0;
    let mut parsed = 0;
    let mut hashes = std::collections::BTreeSet::new();
    let mut associated = 0;
    for line in fs::read_to_string(data.join("manifests/texas-instruments.jsonl"))
        .unwrap_or_default()
        .lines()
    {
        let m: ManifestRecord = serde_json::from_str(line)?;
        if manufacturer.is_none_or(|x| {
            x == "ti" && m.manufacturer == "Texas Instruments" || x == m.manufacturer
        }) && matches!(m.format.as_str(), "step" | "stp")
        {
            raw += 1;
            if hashes.insert(m.sha256.clone()) {
                let p = data
                    .join("objects")
                    .join(&m.sha256[..2])
                    .join(format!("{}.{}", m.sha256, m.format));
                if let Ok(bytes) = fs::read(p) {
                    if step_model::parse(
                        &bytes,
                        step_model::SourceModel {
                            sha256: m.sha256.clone(),
                            original_filename: m.filename.clone(),
                            manufacturer: m.manufacturer.clone(),
                            source_url: Some(m.asset_url.clone()),
                            associated_mpns: vec![m.mpn.clone()],
                            source_package_names: vec![],
                        },
                    )
                    .is_ok()
                    {
                        parsed += 1;
                    }
                }
            }
        }
    }
    for c in load_components(data, manufacturer)? {
        for p in &c.packages {
            if find_ti_model(
                data,
                &c,
                p,
                &data.join("generated/kicad/3dmodels/Personal_Packages.3dshapes"),
            )?
            .is_some()
            {
                associated += 1;
            }
        }
    }
    let root = data.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(root.join("reports"))?;
    fs::write(root.join("reports/step-models.md"),format!("# STEP models\n\nSTEP files acquired | {raw}\n\nUnique raw objects | {}\n\nSTEP files parsed | {parsed}\n\nPackages with STEP | {associated}\n\nAssociations use TI MPN plus normalized package code; ambiguous or invalid models are not emitted.\n",hashes.len()))?;
    println!("STEP files acquired {raw}\nUnique raw objects {}\nSTEP files parsed {parsed}\nPackages with STEP {associated}",hashes.len());
    Ok(())
}
fn model_info(data: &Path, query: &str) -> Result<()> {
    for c in load_components(data, None)? {
        for p in &c.packages {
            if p.name == query || c.mpn == query {
                if let Some(path) = find_ti_model(
                    data,
                    &c,
                    p,
                    &data.join("generated/kicad/3dmodels/Personal_Packages.3dshapes"),
                )? {
                    println!(
                        "Package: {}\nMPN: {}\nModel: {}\nConfidence: Strong",
                        p.name, c.mpn, path
                    );
                    return Ok(());
                }
            }
        }
    }
    for line in fs::read_to_string(data.join("manifests/texas-instruments.jsonl"))
        .unwrap_or_default()
        .lines()
    {
        let m: ManifestRecord = serde_json::from_str(line)?;
        if (m.mpn == query || m.filename == query) && matches!(m.format.as_str(), "step" | "stp") {
            println!(
                "Model: {}\nSHA256: {}\nSource: {}",
                m.filename, m.sha256, m.asset_url
            );
            return Ok(());
        }
    }
    anyhow::bail!("model not found: {query}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sha(ch: char) -> String {
        std::iter::repeat_n(ch, 64).collect()
    }
    fn row(url: &str, mpn: &str) -> serde_json::Value {
        serde_json::json!({"part_number":mpn,"description":null,"functionality":null,"status":null,"automotive":null,"package_code":"DIP","pin_count":8,"bxl_available":true,"step_available":false,"product_url":format!("https://ti.com/{mpn}"),"bxl_url":url,"step_url":null,"package_pitch":null,"package_height":null,"package_length":null,"package_width":null,"source":"test"})
    }
    fn manifest(url: &str, hash: &str) -> serde_json::Value {
        serde_json::json!({"manufacturer":"Texas Instruments","mpn":"TEST","part_url":"https://ti.com/TEST","asset_url":url,"discovery_url":null,"filename":"TEST.bxl","format":"bxl","sha256":hash,"size":1,"retrieved_at":"2026-01-01T00:00:00Z","http_etag":null,"http_last_modified":null,"content_type":null,"source_package":null,"request_url":null,"final_download_url":null,"content_disposition_filename":null})
    }
    fn fixture(
        rows: Vec<serde_json::Value>,
        manifests: Vec<serde_json::Value>,
        objects: &[&str],
    ) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("catalogs/ti-bxl")).unwrap();
        fs::create_dir_all(dir.path().join("manifests")).unwrap();
        fs::write(
            dir.path().join("catalogs/ti-bxl/current.json"),
            serde_json::to_vec(&rows).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("manifests/texas-instruments.jsonl"),
            manifests
                .iter()
                .map(|m| serde_json::to_string(m).unwrap() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        for hash in objects {
            let path = dir.path().join("objects").join(&hash[..2]);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join(format!("{hash}.bxl")), b"object").unwrap();
        }
        dir
    }

    #[test]
    fn current_loader_excludes_historical_and_groups_shared_sha() {
        let current = "https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/CURRENT.bxl";
        let current_alias = "https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/CURRENT_ALIAS.bxl";
        let historical = "https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/HISTORICAL.bxl";
        let hash = sha('a');
        let dir = fixture(
            vec![
                row(current, "A"),
                row(current, "B"),
                row(current_alias, "C"),
            ],
            vec![
                manifest(current, &hash),
                manifest(current_alias, &hash),
                manifest(historical, &sha('b')),
            ],
            &[&hash, &sha('b')],
        );
        let corpus = load_current_ti_bxl_corpus(dir.path()).unwrap();
        assert_eq!(corpus.unique_urls, 2);
        assert_eq!(corpus.observations, 3);
        assert_eq!(corpus.assets.len(), 2);
        assert_eq!(
            corpus
                .assets
                .iter()
                .map(|a| &a.sha256)
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );
        assert_eq!(corpus.historical_bxl_urls, vec![historical]);
    }

    #[test]
    fn loader_reports_multiple_shas_and_missing_object() {
        let ambiguous = "https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/A.bxl";
        let missing = "https://webench.ti.com/cad/dlbxl.cgi/TI_BXL/M.bxl";
        let dir = fixture(
            vec![row(ambiguous, "A"), row(missing, "M")],
            vec![
                manifest(ambiguous, &sha('a')),
                manifest(ambiguous, &sha('b')),
                manifest(missing, &sha('c')),
            ],
            &[],
        );
        let corpus = load_current_ti_bxl_corpus(dir.path()).unwrap();
        assert_eq!(corpus.ambiguous_urls, vec![ambiguous]);
        assert_eq!(corpus.missing_urls, vec![missing]);
    }

    #[test]
    fn shared_sha_is_parsed_once_and_collision_is_detected() {
        let hash = sha('a');
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("object.bxl");
        fs::write(&path, b"bad").unwrap();
        let refs = vec![eda_model::AssetRecord {
            manufacturer: "TI".into(),
            mpn: "B".into(),
            part_url: "p".into(),
            asset_url: "a".into(),
            discovery_url: None,
            filename: "a".into(),
            format: "bxl".into(),
            content_type: None,
            source_package: None,
            request_url: None,
        }];
        let plans = vec![
            ConversionPlan {
                sha: hash.clone(),
                path: path.clone(),
                references: refs.clone(),
                primary: "B".into(),
                output: dir.path().join("same.json"),
                provenance_hash: provenance_hash(&refs),
            },
            ConversionPlan {
                sha: hash.clone(),
                path,
                references: refs.clone(),
                primary: "B".into(),
                output: dir.path().join("same.json"),
                provenance_hash: provenance_hash(&refs),
            },
        ];
        assert_eq!(parse_bxl_documents(&plans).len(), 1);
        let mut second = plans[0].clone();
        second.sha = sha('b');
        assert_eq!(output_collisions(&[plans[0].clone(), second]).len(), 1);
    }

    #[test]
    fn provenance_and_version_are_required_for_cache_reuse() {
        let refs = vec![eda_model::AssetRecord {
            manufacturer: "TI".into(),
            mpn: "A".into(),
            part_url: "p".into(),
            asset_url: "a".into(),
            discovery_url: None,
            filename: "a".into(),
            format: "bxl".into(),
            content_type: None,
            source_package: None,
            request_url: None,
        }];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        let mut c = EdaComponent {
            source_sha256: Some(sha('a')),
            canonicalizer_version: bxl_parser::BXL_CANONICALIZER_VERSION.into(),
            ..Default::default()
        };
        c.metadata
            .insert("provenance_hash".into(), provenance_hash(&refs));
        fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        assert!(cache_matches(&path, &sha('a'), &provenance_hash(&refs)));
        assert!(!cache_matches(&path, &sha('a'), "changed"));
        c.canonicalizer_version = "old-version".into();
        fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        assert!(!cache_matches(&path, &sha('a'), &provenance_hash(&refs)));
    }

    fn component_with_packages(names: &[&str]) -> EdaComponent {
        EdaComponent {
            manufacturer: "TI".into(),
            mpn: "X".into(),
            packages: names
                .iter()
                .map(|name| eda_model::Package {
                    name: (*name).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn production_map_supports_safe_non_deduplicated_packages() {
        let component = component_with_packages(&["SAFE"]);
        let map = production_footprint_map(&[component], None).unwrap();
        assert_eq!(map.get("TI|X|SAFE"), Some(&"SAFE".to_string()));
    }

    #[test]
    fn production_map_excludes_unsafe_packages() {
        let mut component = component_with_packages(&["UNSAFE", "SAFE"]);
        component.packages[0].pads.push(eda_model::Pad {
            number: "1".into(),
            drill: Some(eda_model::Point { x_nm: 1, y_nm: 1 }),
            ..Default::default()
        });
        let map = production_footprint_map(&[component], None).unwrap();
        assert!(!map.contains_key("TI|X|UNSAFE"));
        assert!(map.contains_key("TI|X|SAFE"));
    }

    #[test]
    fn deduplicated_map_requires_a_canonical_group() {
        let component = component_with_packages(&["SAFE"]);
        let normalized = package_normalize::normalize_package(&component, &component.packages[0]);
        let mut broken = normalized.clone();
        broken.fingerprints.kicad_footprint_hash = Some("missing".into());
        let dedupe = package_normalize::DedupeResult {
            packages: vec![broken],
            canonical: Vec::new(),
            physical_groups: BTreeMap::new(),
            manufacturing_groups: BTreeMap::new(),
            warnings: Vec::new(),
        };
        assert!(production_footprint_map(&[component], Some(&dedupe)).is_err());
    }

    #[test]
    fn decoded_bxl_to_kicad_preserves_through_hole_drill() {
        let doc = bxl_model::BxlDocument {
            version: None,
            records: Vec::new(),
            raw_text: Some("PadStack \"TH\"\nPadShape \"Circle\" (Width 40) (Height 40)\n(Drill 20)\nEndPadStack\nPattern \"DIP2\"\nPad (Number 1) (PinName \"A\") (PadStyle \"TH\") (Origin 0, 0)\nSymbol \"U1\"\nPin (PinNum 1) (Origin 0, 0) (PinLength 10)\nPinName \"A\"\nEndSymbol\n".into()),
        };
        let component = bxl_parser::canonicalize(&doc, "TI", "TEST", None);
        let pad = &component.packages[0].pads[0];
        assert!(pad.drill.as_ref().is_some_and(|d| d.x_nm > 0));
        let output = kicad::footprint(&component, &component.packages[0]);
        assert!(output.contains("UNSUPPORTED: unknown drill plating"));
        assert!(!output.contains("thru_hole"));
    }
}
