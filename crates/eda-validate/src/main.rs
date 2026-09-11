use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: CommandLine,
    #[arg(long, default_value = "data")]
    data: PathBuf,
}
#[derive(Subcommand)]
enum CommandLine {
    Ti {
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        refresh_catalog: bool,
        #[arg(long)]
        use_cached_catalog: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        sample: bool,
        #[arg(long)]
        models_only: bool,
        #[arg(long)]
        no_3d: bool,
        #[arg(long)]
        clean: bool,
        #[arg(long)]
        resume: bool,
        #[arg(long)]
        staged: bool,
    },
}
#[derive(Serialize)]
struct Summary {
    manufacturer: String,
    run_id: String,
    limit: Option<usize>,
    dry_run: bool,
    sampling_enabled: bool,
    sampling_algorithm: Option<String>,
    catalog_source: String,
    stages: Vec<Stage>,
    pipeline_version: String,
    dirty_worktree: bool,
    environment: Environment,
}
#[derive(Serialize)]
struct Stage {
    name: String,
    success: bool,
    output: String,
    error: Option<String>,
}
#[derive(Serialize)]
struct Environment {
    os: String,
    arch: String,
    rust: String,
    kicad: String,
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        CommandLine::Ti {
            limit,
            refresh_catalog,
            use_cached_catalog,
            dry_run,
            sample,
            models_only,
            no_3d,
            clean,
            resume,
            staged,
        } => run_ti(
            &cli.data,
            limit,
            refresh_catalog,
            use_cached_catalog,
            dry_run,
            models_only,
            no_3d,
            clean,
            resume,
            staged,
            sample,
        ),
    }
}
#[allow(clippy::too_many_arguments)]
fn run_ti(
    data: &Path,
    limit: Option<usize>,
    refresh: bool,
    cache: bool,
    dry: bool,
    models: bool,
    no3d: bool,
    clean: bool,
    resume: bool,
    staged: bool,
    sample: bool,
) -> Result<()> {
    if staged && !dry {
        run_stage(
            data,
            Some(100),
            refresh,
            cache,
            dry,
            models,
            no3d,
            clean,
            resume,
            sample,
        )?;
        return run_stage(
            data,
            Some(1000),
            false,
            true,
            dry,
            models,
            no3d,
            false,
            true,
            sample,
        );
    }
    run_stage(
        data, limit, refresh, cache, dry, models, no3d, clean, resume, sample,
    )
}
#[allow(clippy::too_many_arguments)]
fn run_stage(
    data: &Path,
    limit: Option<usize>,
    refresh: bool,
    cache: bool,
    dry: bool,
    models: bool,
    no3d: bool,
    clean: bool,
    _resume: bool,
    sample: bool,
) -> Result<()> {
    let root = data
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut stages = Vec::new();
    if refresh {
        stages.push(call(data, "catalog-refresh", &["ti-catalog", "refresh"])?);
    }
    let snap = fs::read(data.join("catalogs/ti/current.json")).unwrap_or_default();
    let mode = if snap.is_empty() { "prefix" } else { "catalog" };
    let mut h = Sha256::new();
    h.update(&snap);
    h.update(
        format!(
            "{:?}{:?}{:?}{:?}:{}",
            limit,
            models,
            no3d,
            sample,
            vendor_ti::SAMPLER_VERSION
        )
        .as_bytes(),
    );
    let id = format!("{:x}", &h.finalize())[..16].to_string();
    let dir = root.join("reports/runs/ti").join(&id);
    fs::create_dir_all(&dir)?;
    let mut fetch = vec!["ti"];
    if !snap.is_empty() || cache {
        fetch.push("--catalog");
    }
    if let Some(n) = limit {
        fetch.push("--limit");
        fetch.push(Box::leak(n.to_string().into_boxed_str()));
    }
    if dry {
        fetch.push("--dry-run");
    }
    if sample {
        fetch.push("--sample");
    }
    if models {
        fetch.push("--models-only");
    }
    fetch.push("--concurrency");
    fetch.push("2");
    stages.push(call(data, "acquisition", &fetch)?);
    if !dry {
        stages.push(call(data, "bxl-convert", &["bxl", "--all"])?);
        stages.push(call(
            data,
            "package-dedupe",
            &["package-dedupe", "--manufacturer", "ti"],
        )?);
        stages.push(call(
            data,
            "model-check",
            &["model-check", "--manufacturer", "ti"],
        )?);
        let mut k = vec![
            "kicad",
            "--manufacturer",
            "ti",
            "--all",
            "--deduplicated",
            "--with-3d",
        ];
        if no3d {
            k.retain(|x| *x != "--with-3d");
        }
        if clean {
            k.push("--clean");
        }
        stages.push(call(data, "kicad-generate", &k)?);
        stages.push(call(
            data,
            "kicad-check",
            &["kicad-check", "data/generated/kicad"],
        )?);
    }
    let env = Environment {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        rust: cmd("rustc", &["--version"]),
        kicad: cmd("kicad-cli", &["--version"]),
    };
    let version = cmd("git", &["rev-parse", "HEAD"]);
    let dirty = !cmd("git", &["status", "--porcelain"]).is_empty();
    let summary = Summary {
        manufacturer: "Texas Instruments".into(),
        run_id: id.clone(),
        limit,
        dry_run: dry,
        sampling_enabled: sample,
        sampling_algorithm: sample.then(|| vendor_ti::SAMPLER_VERSION.into()),
        catalog_source: mode.into(),
        stages,
        pipeline_version: version,
        dirty_worktree: dirty,
        environment: env,
    };
    let json = serde_json::to_string_pretty(&summary)?;
    fs::write(dir.join("summary.json"), format!("{json}\n"))?;
    let mut md = format!("# TI validation\n\nRun ID: `{id}`\n\nCatalog source: `{}`\n\nLimit: {:?}\n\nDry run: {}\n\nSampling enabled: {}\n\nSampling algorithm: `{}`\n\n", summary.catalog_source, limit, dry, sample, summary.sampling_algorithm.as_deref().unwrap_or("none"));
    for s in &summary.stages {
        md.push_str(&format!(
            "* {}: {}\n",
            s.name,
            if s.success { "success" } else { "failed" }
        ));
    }
    fs::write(dir.join("summary.md"), md)?;
    let mut fail = String::new();
    for s in &summary.stages {
        if !s.success {
            fail.push_str(&format!(
                "{{\"stage\":\"{}\",\"class\":\"pipeline\",\"message\":{}}}\n",
                s.name,
                serde_json::to_string(s.error.as_deref().unwrap_or("failed"))?
            ));
        }
    }
    fs::write(dir.join("failures.jsonl"), fail)?;
    fs::create_dir_all(root.join("reports/runs/ti/latest"))?;
    fs::write(
        root.join("reports/runs/ti/latest/summary.json"),
        format!("{json}\n"),
    )?;
    fs::write(
        root.join("reports/runs/ti/latest/summary.md"),
        fs::read(dir.join("summary.md"))?,
    )?;
    println!(
        "TI validation complete\nProducts considered: {:?}\nReport: {}",
        limit,
        dir.join("summary.md").display()
    );
    if summary.stages.iter().any(|s| !s.success) {
        std::process::exit(1)
    }
    Ok(())
}
fn call(data: &Path, name: &str, args: &[&str]) -> Result<Stage> {
    let exe = if name == "catalog-refresh" || name == "acquisition" {
        "eda-fetch"
    } else {
        "eda-convert"
    };
    let bin = std::env::var(format!("EDA_{}_BIN", exe.to_ascii_uppercase())).unwrap_or_else(|_| {
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(exe)
            .to_string_lossy()
            .into()
    });
    let mut command = if Path::new(&bin).exists() {
        let mut c = Command::new(&bin);
        c.args(args);
        c
    } else {
        let mut c = Command::new("cargo");
        c.args(["run", "-q", "-p", exe, "--"]);
        c.args(args);
        c
    };
    let out = command
        .current_dir(
            data.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .output()
        .with_context(|| format!("run {exe}"))?;
    Ok(Stage {
        name: name.into(),
        success: out.status.success(),
        output: String::from_utf8_lossy(&out.stdout).into(),
        error: if out.status.success() {
            None
        } else {
            Some(String::from_utf8_lossy(&out.stderr).into())
        },
    })
}
fn cmd(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .map(|x| String::from_utf8_lossy(&x.stdout).trim().into())
        .unwrap_or_else(|| "unavailable".into())
}
