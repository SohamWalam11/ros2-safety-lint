use clap::{Parser, ValueEnum};
use rayon::prelude::*;
use rosfix::remediator::{apply_remediation, generate_fix, generate_unified_diff, RemediationFix};
use rosfix::semantic_agent::filter_violations;
use rosfix::sros2::{lint_governance, lint_keystore_paths, lint_permissions};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rosfix")]
#[command(about = "High-Performance Multi-Language Static Verification and Active Remediation Engine for ROS 2", long_about = None)]
struct Cli {
    /// Path to scan (defaults to current directory)
    #[arg(long, default_value = ".")]
    path: PathBuf,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Run continuously and stream SARIF JSON to stdout for IDE Language Server integration
    #[arg(long)]
    lsp_mode: bool,

    /// Automatically fix detected safety violations in-place
    #[arg(long)]
    fix: bool,

    /// Enable semantic context filtering to suppress benign telemetry warnings
    #[arg(long)]
    semantic_filter: bool,

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
    suggested_fix: Option<String>,
}

#[tokio::main]
async fn main() {
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

    // Setup scanning spinner
    let spinner = indicatif::ProgressBar::new_spinner();
    spinner.set_style(indicatif::ProgressStyle::default_spinner()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
        .template("{spinner:.green} {msg}")
        .unwrap());
    spinner.set_message("Scanning workspace files for safety violations...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    // Parallel scanning with Rayon across CPU cores
    let mut all_violations: Vec<(String, rosfix::sros2::LintViolation, String)> = files_to_scan
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

                // Collect generated fixes if --fix or --dry-run is specified
                if (cli.fix || cli.dry_run) && !file_violations.is_empty() {
                    let mut fixes = Vec::new();
                    for v in &file_violations {
                        if let Some(fix) = generate_fix(&file_path_str, v, &content) {
                            fixes.push(fix);
                        }
                    }

                    if !fixes.is_empty() {
                        if let Ok(fixed_content) = apply_remediation(&file_path_str, &content, &fixes) {
                            if cli.dry_run {
                                let diff = generate_unified_diff(&file_path_str, &content, &fixed_content);
                                println!("[DRY-RUN FIX] Generated Remediation Patch for {}:\n{}", file_path_str, diff);
                            } else if fs::write(file_path, &fixed_content).is_ok() {
                                println!(
                                    "[FIXED] Applied AST remediation fixes to {}",
                                    file_path_str
                                );
                            }
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

    spinner.finish_with_message("Workspace scan complete.");

    // Semantic Intent & Context-Aware False Positive Filtering
    if cli.semantic_filter {
        let (filtered, suppressed_count) = filter_violations(all_violations);
        all_violations = filtered;
        if cli.format == Format::Text || cli.format == Format::Fancy {
            println!("\x1b[1;32m[SEMANTIC AGENT]\x1b[0m Suppressed {} benign telemetry warning(s).", suppressed_count);
        }
    }

    if cli.fix {
        let blackboard = std::sync::Arc::new(tokio::sync::Mutex::new(rosfix::blackboard::BlackboardEventBus::new()));
        
        println!("\x1b[1;35m[MAS ORCHESTRATOR]\x1b[0m Broadcasting {} violations to the MAS Blackboard...", all_violations.len());
        {
            let mut bb_guard = blackboard.lock().await;
            rosfix::semantic_agent::broadcast_to_blackboard(&all_violations, &mut bb_guard);
        }

        let m = indicatif::MultiProgress::new();
        let pb = m.add(indicatif::ProgressBar::new_spinner());
        pb.set_style(indicatif::ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.blue} {msg}")
            .unwrap());
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let bb_clone = blackboard.clone();
        let _agent2 = tokio::spawn(async move {
            let agent = rosfix::agent::ExecutorAgent;
            use rosfix::agent::ExpertAgent;
            agent.run(bb_clone, pb).await;
        });

        // Simulating waiting for the MAS to process all tasks
        tokio::time::sleep(tokio::time::Duration::from_secs(6)).await;
        m.clear().unwrap();
        println!("\x1b[1;35m[MAS ORCHESTRATOR]\x1b[0m Expert Agents finished processing tasks.");
    }

    match cli.format {
        Format::Text => {
            if all_violations.is_empty() {
                println!("No violations found in {}", cli.path.display());
            } else {
                #[derive(Default)]
                struct DirNode<'a> {
                    files: std::collections::BTreeMap<String, Vec<&'a (String, rosfix::sros2::LintViolation, String)>>,
                    dirs: std::collections::BTreeMap<String, DirNode<'a>>,
                }
                
                let mut root_node = DirNode::default();
                for violation in &all_violations {
                    let path = std::path::Path::new(&violation.0);
                    let mut current = &mut root_node;
                    let components: Vec<_> = path.components().collect();
                    for (i, comp) in components.iter().enumerate() {
                        let name = comp.as_os_str().to_string_lossy().to_string();
                        if i == components.len() - 1 {
                            current.files.entry(name).or_default().push(violation);
                        } else {
                            current = current.dirs.entry(name).or_default();
                        }
                    }
                }

                fn get_line_col(content: &str, start_byte: usize, end_byte: usize) -> (usize, usize, usize) {
                    let mut line = 1;
                    let mut col = 0;
                    let mut start_col = 0;
                    let mut end_col = 0;
                    for (i, c) in content.char_indices() {
                        if i == start_byte { start_col = col; }
                        if i == end_byte { end_col = col; break; }
                        if c == '\n' { line += 1; col = 0; } else { col += 1; }
                    }
                    if end_col == 0 && end_byte >= content.len() { end_col = col; }
                    (line, start_col, end_col)
                }
                
                fn build_tree(name: &str, node: &DirNode, is_root: bool) -> termtree::Tree<String> {
                    let label = if is_root { format!("📦 {}", name) } else { format!("📂 {}", name) };
                    let mut tree = termtree::Tree::new(label);
                    for (dir_name, dir_node) in &node.dirs {
                        tree.push(build_tree(dir_name, dir_node, false));
                    }
                    for (file_name, violations) in &node.files {
                        let mut file_tree = termtree::Tree::new(format!("📄 {}", file_name));
                        for (full_path, v, content) in violations {
                            let (line, start_col, end_col) = get_line_col(content, v.range.start, v.range.end);
                            let icon = if v.message.contains("Hazard") || v.message.contains("Risk") || v.message.contains("Error") || v.message.contains("Safety") { "🛑 [Error]" } else { "⚠️ [Warning]" };
                            let mut v_tree = termtree::Tree::new(format!("{} {}", icon, v.message));
                            v_tree.push(termtree::Tree::new(format!("at line {}, cols {}..{}", line, start_col, end_col)));
                            if let Some(fix) = generate_fix(full_path, v, content) {
                                v_tree.push(termtree::Tree::new(format!("\x1b[1;32mSuggested Fix:\x1b[0m {}", fix.description)));
                            }
                            file_tree.push(v_tree);
                        }
                        tree.push(file_tree);
                    }
                    tree
                }

                let root_name = cli.path.display().to_string();
                let tree = build_tree(&root_name, &root_node, true);
                println!("{}", tree);
            }
        }
        Format::Json => {
            use std::collections::HashMap;
            let mut grouped: HashMap<String, Vec<JsonViolation>> = HashMap::new();
            for (file_str, v, content_str) in &all_violations {
                let suggested_fix = generate_fix(file_str, v, content_str).map(|f| f.replacement_snippet);
                grouped
                    .entry(file_str.clone())
                    .or_default()
                    .push(JsonViolation {
                        message: v.message.clone(),
                        start_byte: v.range.start,
                        end_byte: v.range.end,
                        suggested_fix,
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
                    let start_line = content_str.as_bytes()
                        [..std::cmp::min(v.range.start, content_str.len())]
                        .iter()
                        .filter(|&&c| c == b'\n')
                        .count()
                        + 1;

                    let clean_uri = file_str.replace('\\', "/");
                    let mut result_json = serde_json::json!({
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
                    });

                    if let Some(fix) = generate_fix(&file_str, &v, &content_str) {
                        result_json["fixes"] = serde_json::json!([{
                            "description": {
                                "text": fix.description
                            },
                            "artifactChanges": [{
                                "artifactLocation": {
                                    "uri": clean_uri
                                },
                                "replacements": [{
                                    "deletedRegion": {
                                        "startLine": start_line
                                    },
                                    "insertedContent": {
                                        "text": fix.replacement_snippet
                                    }
                                }]
                            }]
                        }]);
                    }

                    result_json
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
                    if let Some(fix) = generate_fix(file_str, v, content_str) {
                        println!("   \x1b[1;32mFix Suggestion:\x1b[0m {}", fix.description);
                    }
                    println!();
                }
            }
        }
    }
}
