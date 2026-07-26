use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;
use walkdir::WalkDir;

struct BenchmarkResult {
    repo_name: String,
    files_scanned: usize,
    total_time_ms: u128,
    violations_found: usize,
}

fn run_linter(path: &Path) -> (usize, u128) {
    let start_time = Instant::now();
    let output = Command::new("../../target/release/ros2-safety-lint.exe")
        .args(["--format", "json", "--path", path.to_str().unwrap()])
        .output()
        .unwrap();
    let elapsed = start_time.elapsed().as_millis();

    let stdout = String::from_utf8_lossy(&output.stdout);

    // We count the number of JSON violation objects or we just count lines if it's text.
    // Wait, since we are doing --format json, it will print JSON. We can just count the word "message" or parse it.
    let mut violations = 0;
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if let Some(arr) = json.get("violations").and_then(|v| v.as_array()) {
            violations = arr.len();
        }
    }

    (violations, elapsed)
}

fn benchmark_repo(repo_path: &Path, repo_name: &str) -> BenchmarkResult {
    let mut files_scanned = 0;
    let mut total_time_ms = 0;
    let mut violations_found = 0;

    for entry in WalkDir::new(repo_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| !e.file_type().is_dir())
    {
        if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
            if ext == "xml"
                || ext == "yaml"
                || ext == "yml"
                || ext == "py"
                || ext == "urdf"
                || ext == "xacro"
                || ext == "cpp"
                || ext == "hpp"
                || ext == "cc"
                || ext == "c"
                || ext == "h"
            {
                // We now scan XML, YAML, Python, URDF, Xacro, and C++ files
                files_scanned += 1;
                let (v, t) = run_linter(entry.path());
                violations_found += v;
                total_time_ms += t;
            }
        }
    }

    BenchmarkResult {
        repo_name: repo_name.to_string(),
        files_scanned,
        total_time_ms,
        violations_found,
    }
}

fn main() {
    let base_dir = Path::new("tests/fixtures/real_world_datasets");
    let repos = vec!["navigation2", "turtlebot3", "autoware", "moveit2"];

    let mut results = Vec::new();

    println!("Starting Real-World Benchmark...");
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

    for r in results {
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
        println!("");

        csv_content.push_str(&format!(
            "{},{},{},{:.2},{}\n",
            r.repo_name, r.files_scanned, r.total_time_ms, avg_time, r.violations_found
        ));
    }

    fs::write("real_world_benchmark.csv", csv_content).unwrap();
    println!("Exported real_world_benchmark.csv");
}
