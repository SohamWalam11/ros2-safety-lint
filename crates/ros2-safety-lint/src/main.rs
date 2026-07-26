use clap::{Parser, ValueEnum};
use ros2_safety_lint::sros2::{lint_governance, lint_keystore_paths, lint_permissions};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ros2-safety-lint")]
#[command(about = "A static verification linter for ROS 2 QoS and SROS2 security policies", long_about = None)]
struct Cli {
    #[arg(long, value_name = "FILE")]
    path: PathBuf,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Run continuously and stream SARIF JSON to stdout for IDE Language Server integration
    #[arg(long)]
    lsp_mode: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Format {
    Text,
    Json,
    Sarif,
}

#[derive(Serialize)]
struct JsonOutput {
    file: String,
    violations: Vec<JsonViolation>,
}

#[derive(Serialize)]
struct JsonViolation {
    message: String,
    start_byte: usize,
    end_byte: usize,
}

fn main() {
    let cli = Cli::parse();

    let mut files_to_scan = Vec::new();
    if cli.path.is_dir() {
        for entry in walkdir::WalkDir::new(&cli.path).into_iter().filter_map(|e| e.ok()) {
            let path_str = entry.path().to_string_lossy();
            if entry.path().is_file() && !path_str.contains("target") && !path_str.contains(".git") {
                files_to_scan.push(entry.path().to_path_buf());
            }
        }
    } else {
        files_to_scan.push(cli.path.clone());
    }

    // A tuple of (file_path_string, LintViolation)
    let mut all_violations = Vec::new();

    for file_path in files_to_scan {
        if let Some(filename) = file_path.file_name().and_then(|n| n.to_str()) {
            let content = match fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(_) => continue, // Skip binary files or unreadable files
            };

            let mut file_violations = Vec::new();
            let file_path_str = file_path.display().to_string();

            if filename.ends_with(".xml")
                || filename.ends_with(".urdf")
                || filename.ends_with(".xacro")
            {
                if let Ok(doc) = ros2_safety_lint::parser::parse_xml(&content) {
                    if filename == "permissions.xml" {
                        file_violations.extend(lint_permissions(&doc));
                    } else if filename == "governance.xml" {
                        file_violations.extend(lint_governance(&doc));
                    } else if filename == "package.xml" {
                        file_violations
                            .extend(ros2_safety_lint::package_xml_parser::lint_package_xml(&doc));
                    } else if filename.ends_with(".urdf") || filename.ends_with(".xacro") {
                        file_violations.extend(ros2_safety_lint::urdf_parser::lint_urdf(&doc));
                    } else if filename.ends_with(".launch.xml") {
                        file_violations
                            .extend(ros2_safety_lint::launch_xml_parser::lint_launch_xml(&doc));
                    } else {
                        file_violations.extend(lint_permissions(&doc));
                        file_violations.extend(lint_governance(&doc));
                    }
                    file_violations.extend(lint_keystore_paths(&doc));
                }
            } else if filename.ends_with(".yaml") || filename.ends_with(".yml") {
                file_violations.extend(ros2_safety_lint::yaml_parser::lint_yaml(&content));
            } else if filename.ends_with(".py") {
                file_violations.extend(ros2_safety_lint::python_parser::lint_python(&content));
            } else if filename.ends_with(".cpp")
                || filename.ends_with(".hpp")
                || filename.ends_with(".cc")
                || filename.ends_with(".c")
                || filename.ends_with(".h")
            {
                file_violations.extend(ros2_safety_lint::cpp_parser::lint_cpp(&content));
            }

            for v in file_violations {
                all_violations.push((file_path_str.clone(), v, content.clone()));
            }
        }
    }

    match cli.format {
        Format::Text => {
            if all_violations.is_empty() {
                println!("No violations found in {}", cli.path.display());
            } else {
                for (file_str, v, _) in &all_violations {
                    println!("{}: {}", file_str, v.message);
                    println!("  at bytes {}..{}", v.range.start, v.range.end);
                }
            }
        }
        Format::Json => {
            // Group by file
            use std::collections::HashMap;
            let mut grouped: HashMap<String, Vec<JsonViolation>> = HashMap::new();
            for (file_str, v, _) in &all_violations {
                grouped
                    .entry(file_str.clone())
                    .or_default()
                    .push(JsonViolation {
                        message: v.message.clone(),
                        start_byte: v.range.start,
                        end_byte: v.range.end,
                    });
            }

            let mut outputs = Vec::new();
            for (file, violations) in grouped {
                outputs.push(JsonOutput { file, violations });
            }

            match serde_json::to_string_pretty(&outputs) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("Error serializing JSON: {}", e),
            }
        }
        Format::Sarif => {
            let results: Vec<serde_json::Value> = all_violations
                .into_iter()
                .map(|(file_str, v, content_str)| {
                    // Calculate line number from byte offset
                    let start_line = content_str
                        [0..std::cmp::min(v.range.start, content_str.len())]
                        .chars()
                        .filter(|&c| c == '\n')
                        .count()
                        + 1;

                    serde_json::json!({
                        "ruleId": "ros2-safety",
                        "level": "error",
                        "message": {
                            "text": v.message
                        },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": {
                                    "uri": file_str
                                },
                                "region": {
                                    "startLine": start_line
                                }
                            }
                        }]
                    })
                })
                .collect();

            let sarif = serde_json::json!({
                "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
                "version": "2.1.0",
                "runs": [{
                    "tool": {
                        "driver": {
                            "name": "ros2-safety-lint",
                            "informationUri": "https://github.com/ros-security/ros2-safety-lint",
                            "version": "0.1.0"
                        }
                    },
                    "results": results
                }]
            });
            println!("{}", serde_json::to_string_pretty(&sarif).unwrap());
        }
    }
}
