//! `cargo xtask bench`: headless reference benchmarks (TL-04).
//!
//! Builds the editor and runs `orbit-editor --bench <out>` (story load, VM,
//! validation, search index, data screen layout, 2D batching, editor frames
//! and edit-to-error latency on a 10 000-node story), then compares each
//! scenario's median with a baseline. Informational by default: timings on
//! shared CI machines are noisy, so a slower scenario is reported (and
//! annotated on GitHub Actions) without failing, unless `--strict`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Result, cargo, run, workspace_root};

const DEFAULT_OUT: &str = "target/bench/results.json";
const DEFAULT_BASELINE: &str = "xtask/bench-baseline.json";
/// A scenario slower than baseline x this is reported.
const DEFAULT_THRESHOLD: f64 = 1.5;
/// Below this many milliseconds, differences are noise.
const NOISE_FLOOR_MS: f64 = 0.5;

struct Options {
    release: bool,
    quick: bool,
    out: PathBuf,
    baseline: PathBuf,
    threshold: f64,
    strict: bool,
    update_baseline: bool,
}

fn parse(args: &[String]) -> Result<Options> {
    let root = workspace_root();
    let mut o = Options {
        release: true,
        quick: false,
        out: root.join(DEFAULT_OUT),
        baseline: root.join(DEFAULT_BASELINE),
        threshold: DEFAULT_THRESHOLD,
        strict: false,
        update_baseline: false,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} expects a value"))
        };
        match arg.as_str() {
            "--debug" => o.release = false,
            "--quick" => o.quick = true,
            "--strict" => o.strict = true,
            "--update-baseline" => o.update_baseline = true,
            "--out" => o.out = PathBuf::from(value()?),
            "--baseline" => o.baseline = PathBuf::from(value()?),
            "--threshold" => {
                let v = value()?;
                o.threshold = v.parse().map_err(|_| format!("bad threshold `{v}`"))?;
            }
            other => return Err(format!("unknown bench option `{other}`").into()),
        }
    }
    Ok(o)
}

/// One scenario compared with the baseline.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub median_ms: f64,
    pub baseline_ms: Option<f64>,
    /// `median / baseline`.
    pub ratio: Option<f64>,
    pub slower: bool,
}

fn medians(report: &Value) -> Vec<(String, f64)> {
    report["scenarios"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v["median_ms"].as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// Compares two reports (`results` against `baseline`).
pub fn compare(results: &Value, baseline: Option<&Value>, threshold: f64) -> Vec<Row> {
    let base = baseline.map(medians).unwrap_or_default();
    medians(results)
        .into_iter()
        .map(|(name, median_ms)| {
            let baseline_ms = base.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
            let ratio = baseline_ms.filter(|b| *b > 0.0).map(|b| median_ms / b);
            let slower = ratio.is_some_and(|r| r > threshold)
                && median_ms - baseline_ms.unwrap_or(0.0) > NOISE_FLOOR_MS;
            Row {
                name,
                median_ms,
                baseline_ms,
                ratio,
                slower,
            }
        })
        .collect()
}

fn load(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn bench(args: &[String]) -> Result {
    let o = parse(args)?;
    let mut cmd = cargo();
    cmd.args(["run", "-p", "orbit_editor", "--bin", "orbit-editor"]);
    if o.release {
        cmd.arg("--release");
    }
    cmd.arg("--").arg("--bench").arg(&o.out);
    if o.quick {
        cmd.env("ORBIT_BENCH_QUICK", "1");
    }
    run(&mut cmd)?;

    let results = load(&o.out)?;
    if o.update_baseline {
        std::fs::copy(&o.out, &o.baseline)?;
        println!("baseline updated: {}", o.baseline.display());
        return Ok(());
    }
    let baseline = load(&o.baseline).ok();
    if baseline.is_none() {
        println!(
            "no baseline at {} (--update-baseline writes one)",
            o.baseline.display()
        );
    } else if baseline.as_ref().map(|b| &b["build"]) != Some(&results["build"]) {
        println!(
            "note: the baseline is a {} build, these results a {} build",
            baseline
                .as_ref()
                .map_or(Value::Null, |b| b["build"].clone()),
            results["build"]
        );
    }
    let rows = compare(&results, baseline.as_ref(), o.threshold);
    println!(
        "\n{:<28} {:>12} {:>12} {:>8}",
        "scenario", "median ms", "baseline", "ratio"
    );
    let ci = std::env::var_os("GITHUB_ACTIONS").is_some();
    for r in &rows {
        let base = r.baseline_ms.map_or("-".to_owned(), |b| format!("{b:.3}"));
        let ratio = r.ratio.map_or("-".to_owned(), |x| format!("{x:.2}x"));
        let flag = if r.slower { "  SLOWER" } else { "" };
        println!(
            "{:<28} {:>12.3} {:>12} {:>8}{flag}",
            r.name, r.median_ms, base, ratio
        );
        if r.slower && ci {
            println!(
                "::warning title=bench regression::{} is {} slower than the baseline ({:.3} ms vs {base} ms)",
                r.name, ratio, r.median_ms
            );
        }
    }
    println!("\nresults: {}", o.out.display());
    let slower: Vec<&str> = rows
        .iter()
        .filter(|r| r.slower)
        .map(|r| r.name.as_str())
        .collect();
    if o.strict && !slower.is_empty() {
        return Err(format!(
            "slower than the baseline x{}: {}",
            o.threshold,
            slower.join(", ")
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn regressions_are_flagged_above_the_threshold_and_the_noise_floor() {
        let base = json!({"scenarios": {
            "a": {"median_ms": 10.0}, "b": {"median_ms": 10.0}, "c": {"median_ms": 0.1}
        }});
        let now = json!({"scenarios": {
            "a": {"median_ms": 16.0}, "b": {"median_ms": 14.0}, "c": {"median_ms": 0.3},
            "new": {"median_ms": 1.0}
        }});
        let rows = compare(&now, Some(&base), 1.5);
        let get = |n: &str| rows.iter().find(|r| r.name == n).unwrap();
        assert!(get("a").slower);
        assert!(!get("b").slower);
        // 3x, but 0.2 ms: noise.
        assert!(!get("c").slower);
        assert_eq!(get("new").baseline_ms, None);
        assert!(compare(&now, None, 1.5).iter().all(|r| !r.slower));
    }
}
