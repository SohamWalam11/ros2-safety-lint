use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug)]
struct BenchmarkResult {
    repo_name: String,
    files_scanned: usize,
    total_time_ms: u128,
    violations_found: usize,
}

#[derive(serde::Deserialize)]
struct JsonOutput {
    file: String,
    violations: Vec<JsonViolation>,
}

#[derive(serde::Deserialize)]
struct JsonViolation {
    message: String,
}

fn rosfix_binary() -> PathBuf {
    if let Ok(path) = env::var("ROSFIX_BIN") {
        return PathBuf::from(path);
    }
    PathBuf::from("../../target/release/rosfix.exe")
}

fn count_scannable_files(repo_path: &Path) -> usize {
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(repo_path)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let path_str = path.to_string_lossy();
        if path_str.contains("target") || path_str.contains(".git") {
            continue;
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(
                ext,
                "xml" | "yaml" | "yml" | "py" | "urdf" | "xacro" | "cpp" | "hpp" | "cc" | "c" | "h"
            ) {
                count += 1;
            }
        }
    }
    count
}

fn run_linter_on_repo(repo_path: &Path) -> (usize, u128) {
    let start_time = Instant::now();
    let output = Command::new(rosfix_binary())
        .args(["--format", "json", "--path", repo_path.to_str().unwrap()])
        .output()
        .expect("Failed to execute rosfix. Build with: cargo build --release -p rosfix");

    let elapsed = start_time.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut violations = 0usize;
    if let Ok(json) = serde_json::from_str::<Vec<JsonOutput>>(&stdout) {
        for file_output in json {
            violations += file_output.violations.len();
        }
    }

    (violations, elapsed)
}

fn benchmark_repo(repo_path: &Path, repo_name: &str) -> BenchmarkResult {
    let files_scanned = count_scannable_files(repo_path);
    let (violations_found, total_time_ms) = run_linter_on_repo(repo_path);

    BenchmarkResult {
        repo_name: repo_name.to_string(),
        files_scanned,
        total_time_ms,
        violations_found,
    }
}

fn main() {
    let base_dir = Path::new("tests/fixtures/real_world_datasets");
    let repos = vec![
        "navigation2",
        "turtlebot3",
        "autoware",
        "moveit2",
        "PX4-Autopilot",
        "demos",
        "rmf_ros2",
        "rmw_cyclonedds",
        "rmw_fastrtps",
        "ros2_control",
        "sros2",
        "system_tests",
    ];

    let mut results = Vec::new();

    println!("Starting Real-World Benchmark (one rosfix invocation per repository)...");
    for repo in repos {
        println!("Benchmarking {}...", repo);
        let repo_path = base_dir.join(repo);
        if !repo_path.exists() {
            println!("Warning: Repo {} not found. Skipping.", repo);
            continue;
        }
        let result = benchmark_repo(&repo_path, repo);
        results.push(result);
    }

    println!("\n--- Benchmark Results ---");
    let mut csv_content =
        String::from("Repository,FilesScanned,TotalTimeMs,AvgTimePerFileMs,ViolationsFound\n");

    let mut total_files = 0usize;
    let mut total_time = 0u128;
    let mut total_violations = 0usize;

    for r in &results {
        let avg_time = if r.files_scanned > 0 {
            r.total_time_ms as f64 / r.files_scanned as f64
        } else {
            0.0
        };

        println!("Repo: {}", r.repo_name);
        println!("  Files Scanned: {}", r.files_scanned);
        println!("  Total Time: {} ms", r.total_time_ms);
        println!("  Avg Time / File: {:.2} ms", avg_time);
        println!("  Violations Found: {}", r.violations_found);
        println!();

        total_files += r.files_scanned;
        total_time += r.total_time_ms;
        total_violations += r.violations_found;

        csv_content.push_str(&format!(
            "{},{},{},{:.2},{}\n",
            r.repo_name, r.files_scanned, r.total_time_ms, avg_time, r.violations_found
        ));
    }

    let corpus_avg = if total_files > 0 {
        total_time as f64 / total_files as f64
    } else {
        0.0
    };

    println!("--- Corpus Totals ---");
    println!("  Files: {}", total_files);
    println!("  Wall Time (sum of per-repo runs): {} ms", total_time);
    println!("  Avg ms/file (corpus): {:.2}", corpus_avg);
    println!("  Total Violations: {}", total_violations);

    csv_content.push_str(&format!(
        "TOTAL,{},{},{:.2},{}\n",
        total_files, total_time, corpus_avg, total_violations
    ));

    fs::write("real_world_benchmark.csv", csv_content).unwrap();
    println!("Exported real_world_benchmark.csv");
}
