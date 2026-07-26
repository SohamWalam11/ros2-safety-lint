use clap::{Parser, ValueEnum};
use ros2_safety_lint::parser::parse_xml;
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

    let content = match fs::read_to_string(&cli.path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file {}: {}", cli.path.display(), e);
            std::process::exit(1);
        }
    };

    let doc = match parse_xml(&content) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error parsing XML in {}: {}", cli.path.display(), e);
            std::process::exit(1);
        }
    };

    let mut violations = Vec::new();
    if let Some(filename) = cli.path.file_name().and_then(|n| n.to_str()) {
        if filename.ends_with(".xml") || filename.ends_with(".urdf") || filename.ends_with(".xacro")
        {
            let content = fs::read_to_string(&cli.path).unwrap_or_default();
            if let Ok(doc) = ros2_safety_lint::parser::parse_xml(&content) {
                if filename == "permissions.xml" {
                    violations.extend(lint_permissions(&doc));
                } else if filename == "governance.xml" {
                    violations.extend(lint_governance(&doc));
                } else if filename == "package.xml" {
                    violations.extend(ros2_safety_lint::package_xml_parser::lint_package_xml(&doc));
                } else if filename.ends_with(".urdf") || filename.ends_with(".xacro") {
                    violations.extend(ros2_safety_lint::urdf_parser::lint_urdf(&doc));
                } else if filename.ends_with(".launch.xml") {
                    violations.extend(ros2_safety_lint::launch_xml_parser::lint_launch_xml(&doc));
                } else {
                    violations.extend(lint_permissions(&doc));
                    violations.extend(lint_governance(&doc));
                }
                violations.extend(lint_keystore_paths(&doc));
            }
        } else if filename.ends_with(".yaml") || filename.ends_with(".yml") {
            let content = fs::read_to_string(&cli.path).unwrap_or_default();
            violations.extend(ros2_safety_lint::yaml_parser::lint_yaml(&content));
        } else if filename.ends_with(".py") {
            let content = fs::read_to_string(&cli.path).unwrap_or_default();
            violations.extend(ros2_safety_lint::python_parser::lint_python(&content));
        } else if filename.ends_with(".cpp")
            || filename.ends_with(".hpp")
            || filename.ends_with(".cc")
            || filename.ends_with(".c")
            || filename.ends_with(".h")
        {
            let content = fs::read_to_string(&cli.path).unwrap_or_default();
            violations.extend(ros2_safety_lint::cpp_parser::lint_cpp(&content));
        }
    }

    match cli.format {
        Format::Text => {
            if violations.is_empty() {
                println!("No violations found in {}", cli.path.display());
            } else {
                for v in &violations {
                    println!("{}: {}", cli.path.display(), v.message);
                    println!("  at bytes {}..{}", v.range.start, v.range.end);
                }
            }
        }
        Format::Json => {
            let json_violations: Vec<JsonViolation> = violations
                .into_iter()
                .map(|v| JsonViolation {
                    message: v.message,
                    start_byte: v.range.start,
                    end_byte: v.range.end,
                })
                .collect();

            let output = JsonOutput {
                file: cli.path.display().to_string(),
                violations: json_violations,
            };

            match serde_json::to_string_pretty(&output) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("Error serializing JSON: {}", e),
            }
        }
        Format::Sarif => {
            let content_str = fs::read_to_string(&cli.path).unwrap_or_default();

            let results: Vec<serde_json::Value> = violations
                .into_iter()
                .map(|v| {
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
                                    "uri": cli.path.display().to_string()
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
