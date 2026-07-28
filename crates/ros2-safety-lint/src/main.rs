use clap::{Parser, ValueEnum};
use rayon::prelude::*;
use rosfix::sros2::{lint_governance, lint_keystore_paths, lint_permissions};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rosfix")]
#[command(about = "High-Performance Multi-Language Static Verification and Active Remediation Engine for ROS 2", long_about = None)]

struct Cli {
    #[arg(long, value_name = "FILE")]
    path: PathBuf,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Run continuously and stream SARIF JSON to stdout for IDE Language Server integration
    #[arg(long)]
    lsp_mode: bool,

    /// Automatically fix detected safety violations in-place
    #[arg(long)]
    fix: bool,

    /// Preview fixes without modifying files on disk
    #[arg(long)]
    dry_run: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Format {
    Text,
    Json,
    Sarif,
    Fancy,
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
        for entry in walkdir::WalkDir::new(&cli.path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path_str = entry.path().to_string_lossy();
            if entry.path().is_file() && !path_str.contains("target") && !path_str.contains(".git")
            {
                files_to_scan.push(entry.path().to_path_buf());
            }
        }
    } else {
        files_to_scan.push(cli.path.clone());
    }

    // Parallel scanning with Rayon across CPU cores
    let all_violations: Vec<(String, rosfix::sros2::LintViolation, String)> = files_to_scan
        .par_iter()
        .flat_map(|file_path| {
            let mut file_violations = Vec::new();
            if let Some(filename) = file_path.file_name().and_then(|n| n.to_str()) {
                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(_) => return Vec::new(),
                };

                let file_path_str = file_path.display().to_string();

                if filename.ends_with(".xml")
                    || filename.ends_with(".urdf")
                    || filename.ends_with(".xacro")
                {
                    if let Ok(doc) = rosfix::parser::parse_xml(&content) {
                        if filename == "permissions.xml" {
                            file_violations.extend(lint_permissions(&doc));
                        } else if filename == "governance.xml" {
                            file_violations.extend(lint_governance(&doc));
                        } else if filename == "package.xml" {
                            file_violations.extend(
                                rosfix::package_xml_parser::lint_package_xml(&doc),
                            );
                        } else if filename.ends_with(".urdf") || filename.ends_with(".xacro") {
                            file_violations.extend(rosfix::urdf_parser::lint_urdf(&doc));
                        } else if filename.ends_with(".launch.xml") {
                            file_violations.extend(
                                rosfix::launch_xml_parser::lint_launch_xml(&doc),
                            );
                            file_violations.extend(
                                rosfix::lifecycle_parser::lint_lifecycle(&doc),
                            );
                        } else {

                            file_violations.extend(lint_permissions(&doc));
                            file_violations.extend(lint_governance(&doc));
                        }
                        file_violations.extend(lint_keystore_paths(&doc));
                    }
                } else if filename.ends_with(".yaml") || filename.ends_with(".yml") {
                    file_violations.extend(rosfix::yaml_parser::lint_yaml(&content));
                } else if filename.ends_with(".py") {
                    file_violations.extend(rosfix::python_parser::lint_python(&content));
                } else if filename.ends_with(".cpp")
                    || filename.ends_with(".hpp")
                    || filename.ends_with(".cc")
                    || filename.ends_with(".c")
                    || filename.ends_with(".h")
                {
                    file_violations.extend(rosfix::cpp_parser::lint_cpp(&content));
                }


                if cli.fix && !file_violations.is_empty() {
                    let mut fixed_content = content.clone();
                    let mut modified = false;

                    if filename.ends_with(".launch.xml") {
                        if fixed_content.contains("<node ") && !fixed_content.contains("respawn=") {
                            fixed_content =
                                fixed_content.replace("<node ", "<node respawn=\"true\" ");
                            modified = true;
                        }
                    } else if filename == "governance.xml" {
                        if fixed_content.contains("<rtps_protection_kind>NONE</rtps_protection_kind>")
                        {
                            fixed_content = fixed_content.replace(
                                "<rtps_protection_kind>NONE</rtps_protection_kind>",
                                "<rtps_protection_kind>ENCRYPT</rtps_protection_kind>",
                            );
                            modified = true;
                        } else if fixed_content
                            .contains("<rtps_protection_kind>SIGN</rtps_protection_kind>")
                        {
                            fixed_content = fixed_content.replace(
                                "<rtps_protection_kind>SIGN</rtps_protection_kind>",
                                "<rtps_protection_kind>ENCRYPT</rtps_protection_kind>",
                            );
                            modified = true;
                        }
                    } else if filename == "package.xml" {
                        if fixed_content.contains("format=\"1\"")
                            || fixed_content.contains("format=\"2\"")
                        {
                            fixed_content = fixed_content
                                .replace("format=\"1\"", "format=\"3\"")
                                .replace("format=\"2\"", "format=\"3\"");
                            modified = true;
                        }
                        if !fixed_content.contains("<license>") {
                            fixed_content = fixed_content.replace(
                                "</package>",
                                "  <license>Apache-2.0</license>\n</package>",
                            );
                            modified = true;
                        }
                    } else if filename.ends_with(".yaml") || filename.ends_with(".yml") {
                        if fixed_content.contains("robot_radius: 0.0") {
                            fixed_content = fixed_content.replace("robot_radius: 0.0", "robot_radius: 0.25");
                            modified = true;
                        }
                        if fixed_content.contains("ROS_DOMAIN_ID: 0") {
                            fixed_content = fixed_content.replace("ROS_DOMAIN_ID: 0", "ROS_DOMAIN_ID: 42");
                            modified = true;
                        }
                    } else if filename.ends_with(".urdf") || filename.ends_with(".xacro") {
                        if fixed_content.contains("<joint ") && !fixed_content.contains("<limit ") {
                            fixed_content = fixed_content.replace(
                                "<joint ",
                                "<joint ",
                            );
                        }
                    }


                    if modified {
                        if cli.dry_run {
                            println!("[DRY-RUN FIX] Would remediate violations in {}", file_path_str);
                        } else if fs::write(file_path, &fixed_content).is_ok() {
                            println!(
                                "[FIXED] Applied automatic safety remediation to {}",
                                file_path_str
                            );
                        }
                    }
                }

                file_violations
                    .into_iter()
                    .map(|v| (file_path_str.clone(), v, content.clone()))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        })
        .collect();



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
                    let start_line = content_str.as_bytes()
                        [..std::cmp::min(v.range.start, content_str.len())]
                        .iter()
                        .filter(|&&c| c == b'\n')
                        .count()
                        + 1;

                    let clean_uri = file_str.replace('\\', "/");
                    serde_json::json!({
                        "ruleId": "rosfix",
                        "level": "error",
                        "message": {
                            "text": v.message
                        },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": {
                                    "uri": clean_uri
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
                            "name": "rosfix",
                            "informationUri": "https://github.com/ros-security/rosfix",
                            "version": "0.1.0",
                            "rules": [{
                                "id": "rosfix",
                                "name": "ROS2SafetyRule",
                                "shortDescription": {
                                    "text": "ROS 2 Static Safety Violation"
                                },
                                "fullDescription": {
                                    "text": "Detects QoS incompatibilities, SROS2 security flaws, and physical safety risks."
                                }
                            }]
                        }
                    },
                    "results": results
                }]
            });
            println!("{}", serde_json::to_string_pretty(&sarif).unwrap());
        }
        Format::Fancy => {
            if all_violations.is_empty() {
                println!("No violations found in {}", cli.path.display());
            } else {
                for (file_str, v, content_str) in &all_violations {
                    let start_line = content_str.as_bytes()
                        [..std::cmp::min(v.range.start, content_str.len())]
                        .iter()
                        .filter(|&&c| c == b'\n')
                        .count()
                        + 1;
                    println!("\x1b[1;31mError [rosfix]\x1b[0m: {}", v.message);
                    println!("  \x1b[1;34m-->\x1b[0m {}:{}", file_str, start_line);
                    println!("   \x1b[1;34m|\x1b[0m");
                    println!(" \x1b[1;34m{:>3} |\x1b[0m  {}", start_line, v.message);
                    println!("   \x1b[1;34m|\x1b[0m  \x1b[1;31m^^^^^^^^^^^^^^^^\x1b[0m");
                    println!();
                }
            }
        }

    }
}

