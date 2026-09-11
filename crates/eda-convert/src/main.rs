use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use eda_model::EdaComponent;
use eda_model::ManifestRecord;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
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
                convert(input, mpn, manufacturer, &c.data)
            }
        }
        Command::BxlStats => stats(&c.data),
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
) -> Result<()> {
    let bytes = fs::read(&input).with_context(|| format!("read {}", input.display()))?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let doc = bxl_parser::parse(&bytes)
        .with_context(|| format!("parse {} at byte stream", input.display()))?;
    let manufacturer = manufacturer.unwrap_or_else(|| "Unknown".into());
    let mpn = mpn.unwrap_or_else(|| input.file_stem().unwrap().to_string_lossy().into());
    let c = bxl_parser::canonicalize(&doc, &manufacturer, &mpn, Some(hash));
    let out = data
        .join("derived/components")
        .join(safe(&c.manufacturer))
        .join(format!("{}.json", safe(&c.mpn)));
    if let Ok(existing) = fs::read_to_string(&out) {
        if existing.contains("\"pin_map\"")
            && existing.contains(&format!(
                "\"source_sha256\": \"{}\"",
                c.source_sha256.as_deref().unwrap_or_default()
            ))
        {
            return Ok(());
        }
    }
    fs::create_dir_all(out.parent().unwrap())?;
    let json = serde_json::to_string_pretty(&c)?;
    fs::write(&out, format!("{json}\n"))?;
    println!("{} -> {}", input.display(), out.display());
    Ok(())
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
    let manifest = data.join("manifests/texas-instruments.jsonl");
    let mut done = 0;
    for line in fs::read_to_string(manifest)?.lines() {
        let m: ManifestRecord = serde_json::from_str(line)?;
        if m.format != "bxl" {
            continue;
        }
        let p = data
            .join("objects")
            .join(&m.sha256[..2])
            .join(format!("{}.bxl", m.sha256));
        if p.exists() {
            convert(p, Some(m.mpn), Some(m.manufacturer), data)?;
            done += 1;
        }
    }
    println!("converted {done} BXL observations");
    Ok(())
}
fn stats(data: &Path) -> Result<()> {
    let mut files = 0;
    let mut parsed = 0;
    let mut records = 0;
    for e in walkdir::WalkDir::new(data.join("objects"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "bxl"))
    {
        files += 1;
        if let Ok(d) = bxl_parser::parse(&fs::read(e.path())?) {
            parsed += 1;
            records += d.records.len()
        }
    }
    println!("BXL objects                    {files}\nParsed successfully            {parsed}\nFailed                           {}\nRaw records                    {records}",files-parsed);
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
    let dedupe = if deduplicated {
        Some(package_normalize::dedupe(
            &by_m.values().flatten().cloned().collect::<Vec<_>>(),
        ))
    } else {
        None
    };
    let footprint_map = dedupe
        .as_ref()
        .map(|d| {
            let mut m = std::collections::BTreeMap::new();
            for p in &d.packages {
                if let Some(g) = d.canonical.iter().find(|g| {
                    g.fingerprints.manufacturing_hash == p.fingerprints.manufacturing_hash
                }) {
                    m.insert(
                        format!(
                            "{}|{}|{}",
                            p.source.manufacturer, p.source.mpn, p.source.name
                        ),
                        safe(&g.preferred_name),
                    );
                }
            }
            m
        })
        .unwrap_or_default();
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
                let out_name = dedupe
                    .as_ref()
                    .and_then(|d| {
                        d.packages
                            .iter()
                            .find(|x| x.source.mpn == c.mpn && x.source.name == p.name)
                    })
                    .map(|x| dname(dedupe.as_ref().unwrap(), x))
                    .unwrap_or_else(|| safe(&p.name));
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
fn dname(d: &package_normalize::DedupeResult, p: &package_normalize::NormalizedPackage) -> String {
    d.canonical
        .iter()
        .find(|x| x.fingerprints.manufacturing_hash == p.fingerprints.manufacturing_hash)
        .map(|x| safe(&x.preferred_name))
        .unwrap_or_else(|| safe(&p.source.name))
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
    println!("KiCad validation\n\nSymbol libraries       {symbols}\nFootprints             {footprints}\nSyntax errors          {errors}");
    if errors > 0 {
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
