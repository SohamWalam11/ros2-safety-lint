use roxmltree::Document;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
enum Tier {
    Tier1, // Obvious
    Tier2, // Subtle
    Tier3, // Adversarial
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Tier::Tier1 => "Tier1 (Obvious)",
            Tier::Tier2 => "Tier2 (Subtle)",
            Tier::Tier3 => "Tier3 (Adversarial)",
        };
        write!(f, "{}", s)
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct GroundTruth {
    injected_bugs: HashMap<String, Vec<InjectedBug>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct InjectedBug {
    tier: Tier,
    bug_type: String,
}

#[derive(Deserialize, Debug)]
struct LinterOutput {
    file: String,
    violations: Vec<LinterViolation>,
}

#[derive(Deserialize, Debug)]
struct LinterViolation {
    message: String,
}

fn mutate_file(clean_path: &Path, mutated_path: &Path) -> Vec<InjectedBug> {
    let content = fs::read_to_string(clean_path).unwrap();
    let doc = Document::parse(&content).unwrap();
    let mut bugs = Vec::new();
    let mut new_content = content.clone();

    let filename = clean_path.file_name().unwrap().to_str().unwrap();

    if filename == "permissions.xml" {
        // Tier 3: Adversarial
        for node in doc.descendants() {
            if node.has_tag_name("subject_name") {
                if let Some(text) = node.text() {
                    new_content = content.replace(text, "*");
                    bugs.push(InjectedBug {
                        tier: Tier::Tier3,
                        bug_type: "WildcardSubject".to_string(),
                    });
                    break;
                }
            }
        }
    } else if filename == "governance.xml" {
        // Tier 1: Obvious
        for node in doc.descendants() {
            if node.has_tag_name("rtps_protection_kind") {
                if let Some(text) = node.text() {
                    new_content = content.replace(text, "NONE");
                    bugs.push(InjectedBug {
                        tier: Tier::Tier1,
                        bug_type: "DowngradedRtps".to_string(),
                    });
                    break;
                }
            }
        }
    } else if filename == "demo.launch.xml" {
        // Tier 2: Subtle
        for node in doc.descendants() {
            if node.has_tag_name("env") {
                if let Some(value) = node.attribute("value") {
                    new_content = content.replace(value, "/absolute/keystore");
                    bugs.push(InjectedBug {
                        tier: Tier::Tier2,
                        bug_type: "AbsoluteKeystorePath".to_string(),
                    });
                    break;
                }
            }
        }
    }

    fs::write(mutated_path, new_content).unwrap();
    bugs
}

#[derive(Default)]
struct Metrics {
    tp: usize,
    fp: usize,
    fn_count: usize,
    tn: usize,
}

impl Metrics {
    fn precision(&self) -> f64 {
        if self.tp + self.fp == 0 {
            1.0
        } else {
            self.tp as f64 / (self.tp + self.fp) as f64
        }
    }
    fn recall(&self) -> f64 {
        if self.tp + self.fn_count == 0 {
            1.0
        } else {
            self.tp as f64 / (self.tp + self.fn_count) as f64
        }
    }
}

fn evaluate(clean_dir: &Path, mutated_dir: &Path, ground_truth: &GroundTruth) {
    let mut tier_metrics: HashMap<Tier, Metrics> = HashMap::new();
    let mut all_metrics = Metrics::default();

    for entry in fs::read_dir(mutated_dir).unwrap() {
        let path = entry.unwrap().path();
        let output = run_linter(&path);
        let filename = path.file_name().unwrap().to_str().unwrap();
        let expected_bugs = ground_truth.injected_bugs.get(filename).unwrap();

        if expected_bugs.is_empty() {
            if output.violations.is_empty() {
                all_metrics.tn += 1;
            } else {
                all_metrics.fp += output.violations.len();
            }
        } else {
            let tier = expected_bugs[0].tier.clone();
            let tm = tier_metrics.entry(tier).or_insert(Metrics::default());

            if output.violations.is_empty() {
                tm.fn_count += expected_bugs.len();
                all_metrics.fn_count += expected_bugs.len();
            } else {
                tm.tp += expected_bugs.len();
                all_metrics.tp += expected_bugs.len();
                if output.violations.len() > expected_bugs.len() {
                    let fp_diff = output.violations.len() - expected_bugs.len();
                    tm.fp += fp_diff;
                    all_metrics.fp += fp_diff;
                }
            }
        }
    }

    for entry in fs::read_dir(clean_dir).unwrap() {
        let path = entry.unwrap().path();
        let output = run_linter(&path);
        if output.violations.is_empty() {
            all_metrics.tn += 1;
        } else {
            all_metrics.fp += output.violations.len();
        }
    }

    println!("--- Evaluation Results (Tiered) ---");
    let mut csv = String::from("Tier,TP,FP,FN,Precision,Recall\n");

    for (tier, tm) in &tier_metrics {
        println!(
            "{}: Precision: {:.2}%, Recall: {:.2}%",
            tier,
            tm.precision() * 100.0,
            tm.recall() * 100.0
        );
        csv.push_str(&format!(
            "{},{},{},{},{:.2},{:.2}\n",
            tier,
            tm.tp,
            tm.fp,
            tm.fn_count,
            tm.precision(),
            tm.recall()
        ));
    }

    println!(
        "Overall: Precision: {:.2}%, Recall: {:.2}%",
        all_metrics.precision() * 100.0,
        all_metrics.recall() * 100.0
    );
    csv.push_str(&format!(
        "All,{},{},{},{:.2},{:.2}\n",
        all_metrics.tp,
        all_metrics.fp,
        all_metrics.fn_count,
        all_metrics.precision(),
        all_metrics.recall()
    ));

    fs::write("evaluation_results_tiered.csv", csv).unwrap();
}

fn run_linter(path: &Path) -> LinterOutput {
    let output = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "ros2-safety-lint",
            "--",
            "--format",
            "json",
            "--path",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    serde_json::from_str(&stdout).unwrap_or_else(|_| LinterOutput {
        file: path.to_str().unwrap().to_string(),
        violations: vec![],
    })
}

fn main() {
    let clean_dir = PathBuf::from("tests/fixtures/clean");
    let mutated_dir = PathBuf::from("tests/fixtures/mutated");

    fs::create_dir_all(&mutated_dir).unwrap();

    let mut ground_truth = GroundTruth {
        injected_bugs: HashMap::new(),
    };

    println!("Generating Tiered Mutants...");
    for entry in fs::read_dir(&clean_dir).unwrap() {
        let entry = entry.unwrap();
        let clean_path = entry.path();
        let filename = clean_path.file_name().unwrap().to_str().unwrap();
        let mutated_path = mutated_dir.join(filename);

        let bugs = mutate_file(&clean_path, &mutated_path);
        ground_truth
            .injected_bugs
            .insert(filename.to_string(), bugs);
    }

    let gt_json = serde_json::to_string_pretty(&ground_truth).unwrap();
    fs::write("ground_truth.json", gt_json).unwrap();

    println!("Evaluating Linter Across Tiers...");
    evaluate(&clean_dir, &mutated_dir, &ground_truth);
}
