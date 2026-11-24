//! A deliberately small CMake reader: enough to know which executables a ROS
//! package builds and installs, which packages it finds, and which directories it
//! installs into `share/<pkg>`. Anything it cannot follow marks the result as
//! incomplete so dependent checks stay silent.

use crate::facts::{Executable, PackageUse, RosidlCall, UseKind};
use crate::source::{FileId, Loc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub args: Vec<(String, usize, usize)>,
    pub start: usize,
    pub end: usize,
    /// Nesting depth inside foreach/function/macro/while blocks.
    pub loop_depth: usize,
}

/// Splits CMake source into commands with raw (unexpanded) arguments.
pub fn commands(src: &str) -> Vec<Command> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut loop_depth = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'#' {
            if src[i..].starts_with("#[[") || src[i..].starts_with("#[=") {
                if let Some(e) = src[i..].find("]]") {
                    i += e + 2;
                    continue;
                }
            }
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let name = src[start..i].to_ascii_lowercase();
            let mut j = i;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            if j < b.len() && b[j] == b'(' {
                let (args, end) = parse_args(src, j + 1);
                match name.as_str() {
                    "foreach" | "function" | "macro" | "while" => loop_depth += 1,
                    "endforeach" | "endfunction" | "endmacro" | "endwhile" => {
                        loop_depth = loop_depth.saturating_sub(1)
                    }
                    _ => {}
                }
                out.push(Command {
                    name,
                    args,
                    start,
                    end,
                    loop_depth,
                });
                i = end;
                continue;
            }
            continue;
        }
        i += 1;
    }
    out
}

fn parse_args(src: &str, mut i: usize) -> (Vec<(String, usize, usize)>, usize) {
    let b = src.as_bytes();
    let mut args = Vec::new();
    let mut depth = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b')' if depth == 0 => return (args, i + 1),
            b')' => {
                depth -= 1;
                i += 1;
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                let s = i + 1;
                let mut j = s;
                while j < b.len() && b[j] != b'"' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                args.push((src[s..j.min(b.len())].to_string(), i, (j + 1).min(b.len())));
                i = j + 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                let s = i;
                while i < b.len()
                    && !b[i].is_ascii_whitespace()
                    && b[i] != b'('
                    && b[i] != b')'
                    && b[i] != b'"'
                {
                    i += 1;
                }
                args.push((src[s..i].to_string(), s, i));
            }
        }
    }
    (args, b.len())
}

#[derive(Debug, Default)]
pub struct CmakeFacts {
    pub executables: Vec<Executable>,
    pub complete: bool,
    pub share_dirs: Vec<String>,
    pub install_complete: bool,
    pub uses: Vec<PackageUse>,
    pub param_specs: Vec<String>,
    pub components: Vec<String>,
    pub rosidl: Option<RosidlCall>,
    /// Libraries whose classes are registered as components but that are never installed
    /// (only filled when the install rules were fully understood).
    pub uninstalled_component_libs: Vec<(String, Loc)>,
    /// `pluginlib_export_plugin_description_file(<base_pkg> <xml>)`: plugin XMLs (package-relative).
    pub plugin_xmls: Vec<String>,
    /// Literal source paths of `install(FILES|PROGRAMS|DIRECTORY …)` (workspace-relative),
    /// recorded only when every argument was understood and the install is not OPTIONAL.
    pub install_sources: Vec<(String, Loc)>,
}

const KEYWORDS: &[&str] = &[
    "WIN32",
    "MACOSX_BUNDLE",
    "EXCLUDE_FROM_ALL",
    "STATIC",
    "SHARED",
    "MODULE",
    "OBJECT",
    "INTERFACE",
    "IMPORTED",
    "ALIAS",
    "PUBLIC",
    "PRIVATE",
];

/// Interprets the commands of a package's CMakeLists.txt.
/// `pkg_dir` is the workspace-relative package directory (used to resolve source paths).
pub fn interpret(file: FileId, src: &str, pkg_name: &str, pkg_dir: &str) -> CmakeFacts {
    let cmds = commands(src);
    let mut vars: HashMap<String, Vec<String>> = HashMap::new();
    vars.insert("PROJECT_NAME".into(), vec![pkg_name.to_string()]);
    vars.insert("CMAKE_CURRENT_SOURCE_DIR".into(), vec![String::new()]);
    vars.insert("CMAKE_INSTALL_PREFIX".into(), vec![String::new()]);
    let mut f = CmakeFacts {
        complete: true,
        install_complete: true,
        ..Default::default()
    };
    // target -> (sources, is_executable, loc)
    let mut targets: Vec<(String, Vec<String>, bool, Loc)> = Vec::new();
    let mut links: Vec<(String, Vec<String>)> = Vec::new();
    let mut installed: Vec<String> = Vec::new();
    let mut ament_auto = false;
    let mut component_libs: Vec<(String, Loc)> = Vec::new();
    // Every target named in any `install(TARGETS …)` (libraries go to `lib`, not `lib/<pkg>`).
    let mut installed_any: Vec<String> = Vec::new();

    let rel = |p: &str| -> String {
        let p = p.trim_start_matches("./");
        if pkg_dir.is_empty() {
            p.to_string()
        } else {
            format!("{pkg_dir}/{p}")
        }
    };

    for c in &cmds {
        let expand = |s: &str, vars: &HashMap<String, Vec<String>>| -> Option<Vec<String>> {
            expand_var(s, vars)
        };
        // Expand all args; unknown variables become None.
        let ex: Vec<Option<Vec<String>>> =
            c.args.iter().map(|(a, _, _)| expand(a, &vars)).collect();
        let flat = |from: usize| -> (Vec<String>, bool) {
            let mut v = Vec::new();
            let mut ok = true;
            for e in ex.iter().skip(from) {
                match e {
                    Some(list) => v.extend(list.iter().cloned()),
                    None => ok = false,
                }
            }
            (v, ok)
        };
        let loc = Loc::new(file, c.start, c.end);
        match c.name.as_str() {
            "project" => {
                if let Some(Some(n)) = ex.first() {
                    if let Some(n) = n.first() {
                        vars.insert("PROJECT_NAME".into(), vec![n.clone()]);
                    }
                }
            }
            "set" => {
                if let Some((name, _, _)) = c.args.first() {
                    let (vals, ok) = flat(1);
                    let vals: Vec<String> = vals
                        .into_iter()
                        .filter(|v| v != "CACHE" && v != "PARENT_SCOPE")
                        .collect();
                    if ok && c.loop_depth == 0 {
                        vars.insert(name.clone(), vals);
                    } else {
                        vars.remove(name);
                    }
                }
            }
            "list" => {
                // list(APPEND VAR a b)
                if c.args.len() >= 2 && c.args[0].0.eq_ignore_ascii_case("APPEND") {
                    let name = c.args[1].0.clone();
                    let (vals, ok) = flat(2);
                    if ok && c.loop_depth == 0 {
                        vars.entry(name).or_default().extend(vals);
                    } else {
                        vars.remove(&name);
                    }
                }
            }
            "add_executable"
            | "add_library"
            | "ament_auto_add_executable"
            | "ament_auto_add_library"
            | "cuda_add_executable"
            | "cuda_add_library" => {
                let is_exe = c.name.contains("executable");
                if c.name.starts_with("ament_auto") {
                    ament_auto = true;
                }
                let Some(Some(name)) = ex.first() else {
                    f.complete = false;
                    continue;
                };
                let Some(name) = name.first().cloned() else {
                    continue;
                };
                if c.loop_depth > 0 {
                    f.complete = false;
                }
                let (srcs, _) = flat(1);
                let sources: Vec<String> = srcs
                    .iter()
                    .filter(|s| !KEYWORDS.contains(&s.as_str()) && looks_like_source(s))
                    .map(|s| rel(s))
                    .collect();
                // ament_auto targets are installed automatically.
                if c.name.starts_with("ament_auto") && is_exe {
                    installed.push(name.clone());
                }
                targets.push((name, sources, is_exe, loc));
            }
            "target_link_libraries" | "ament_target_dependencies" => {
                if let Some(Some(t)) = ex.first() {
                    if let Some(t) = t.first() {
                        let (libs, _) = flat(1);
                        if c.name == "ament_target_dependencies" {
                            for l in &libs {
                                if !KEYWORDS.contains(&l.as_str()) && l != "SYSTEM" {
                                    f.uses.push(PackageUse {
                                        name: l.clone(),
                                        kind: UseKind::CmakeFindPackage,
                                        loc,
                                    });
                                }
                            }
                        }
                        links.push((t.clone(), libs));
                    }
                }
            }
            "find_package" => {
                if let Some(Some(n)) = ex.first() {
                    if let Some(n) = n.first() {
                        f.uses.push(PackageUse {
                            name: n.clone(),
                            kind: UseKind::CmakeFindPackage,
                            loc,
                        });
                    }
                }
            }
            "install" => {
                let (args, ok) = flat(0);
                if !ok || c.loop_depth > 0 {
                    f.install_complete = false;
                }
                if args.first().is_some_and(|m| m == "TARGETS") {
                    installed_any.extend(args.iter().skip(1).take_while(|a| !is_install_keyword(a)).cloned());
                }
                let source_mode = args.first().is_some_and(|m| matches!(m.as_str(), "FILES" | "PROGRAMS" | "DIRECTORY"));
                // Only fully literal rules: variables may be set in `if()` branches this
                // interpreter does not follow, or name files generated at configure time.
                let literal = c
                    .args
                    .iter()
                    .skip(1)
                    .take_while(|(a, _, _)| !is_install_keyword(a))
                    .all(|(a, _, _)| !a.contains('$'));
                if source_mode && literal && ok && c.loop_depth == 0 && !args.iter().any(|a| a == "OPTIONAL") {
                    for item in args.iter().skip(1).take_while(|a| !is_install_keyword(a)) {
                        // Absolute paths (or ones built from unknown directories) and
                        // generator expressions are not ours to resolve.
                        let p = item.trim_end_matches('/');
                        if p.is_empty() || p.starts_with('/') || p.contains(':') || p.contains("$<") || p.contains('*') {
                            continue;
                        }
                        // Mentioned by another command (file(DOWNLOAD …), configure_file,
                        // add_custom_command OUTPUT …): created during the build.
                        let elsewhere = cmds.iter().any(|o| {
                            o.name != "install" && o.args.iter().any(|(a, _, _)| a.contains(&format!("/{p}")) || a == p || a.starts_with(&format!("{p}/")))
                        });
                        if elsewhere {
                            continue;
                        }
                        f.install_sources.push((rel(p), loc));
                    }
                }
                interpret_install(
                    &args,
                    pkg_name,
                    &mut installed,
                    &mut f,
                    &mut targets,
                    loc,
                    &rel,
                );
            }
            "rclcpp_components_register_node" => {
                // rclcpp_components_register_node(<lib> PLUGIN "<cls>" EXECUTABLE <exe>)
                let (args, _) = flat(0);
                let lib = args.first().cloned().unwrap_or_default();
                component_libs.push((lib.clone(), loc));
                let mut exe = None;
                let mut it = args.iter();
                while let Some(a) = it.next() {
                    if a == "EXECUTABLE" {
                        exe = it.next().cloned();
                    } else if a == "PLUGIN" {
                        if let Some(p) = it.next() {
                            f.components.push(p.clone());
                        }
                    }
                }
                let lib_sources = targets
                    .iter()
                    .find(|t| t.0 == lib)
                    .map(|t| t.1.clone())
                    .unwrap_or_default();
                if let Some(e) = exe {
                    targets.push((e.clone(), lib_sources, true, loc));
                    installed.push(e);
                }
            }
            "rclcpp_components_register_nodes" => {
                if let Some(Some(l)) = ex.first() {
                    if let Some(lib) = l.first() {
                        component_libs.push((lib.clone(), loc));
                    }
                }
                let (args, _) = flat(1);
                f.components
                    .extend(args.into_iter().filter(|a| a.contains("::")));
            }
            "pluginlib_export_plugin_description_file" => {
                if let Some(Some(xml)) = ex.get(1) {
                    if let Some(x) = xml.first() {
                        f.plugin_xmls.push(x.trim_start_matches("${CMAKE_CURRENT_SOURCE_DIR}/").trim_start_matches('/').to_string());
                    }
                }
            }
            "rosidl_generate_interfaces" => {
                // rosidl_generate_interfaces(<target> "msg/A.msg" ... DEPENDENCIES ... ADD_LINTER_TESTS)
                const STOP: &[&str] = &["DEPENDENCIES", "LIBRARY_NAME", "ADD_LINTER_TESTS", "SKIP_INSTALL", "SKIP_GROUP_MEMBERSHIP_CHECK"];
                let mut complete = true;
                let mut files = Vec::new();
                let mut skip_group = false;
                let mut in_files = true;
                for e in ex.iter().skip(1) {
                    let Some(list) = e else {
                        if in_files {
                            complete = false;
                        }
                        continue;
                    };
                    for a in list {
                        if a == "SKIP_GROUP_MEMBERSHIP_CHECK" {
                            skip_group = true;
                        }
                        if STOP.contains(&a.as_str()) {
                            in_files = false;
                            continue;
                        }
                        if in_files {
                            // `${CMAKE_CURRENT_SOURCE_DIR}:msg/A.msg` style: keep the relative part.
                            let rel = a.rsplit(':').next().unwrap_or(a).trim_start_matches('/');
                            if rel.contains('*') || rel.contains("${") {
                                complete = false;
                            } else if !rel.is_empty() {
                                files.push(rel.to_string());
                            }
                        }
                    }
                }
                f.rosidl = Some(RosidlCall { files, complete, skip_group_check: skip_group, loc });
            }
            "generate_parameter_library" => {
                if let Some(Some(spec)) = ex.get(1) {
                    if let Some(s) = spec.first() {
                        f.param_specs.push(rel(s));
                    }
                }
            }
            "ament_auto_package" => {
                ament_auto = true;
                let (args, _) = flat(0);
                let mut in_share = false;
                for a in args {
                    if a == "INSTALL_TO_SHARE" {
                        in_share = true;
                        continue;
                    }
                    if a.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                        in_share = false;
                        continue;
                    }
                    if in_share {
                        f.share_dirs.push(a.trim_end_matches('/').to_string());
                    }
                }
            }
            "include" | "add_subdirectory"
                // Content we cannot see may define or install targets.
                if !c
                    .args
                    .first()
                    .is_some_and(|a| a.0.starts_with("CTest") || a.0.starts_with("GNUInstallDirs"))
                => {
                    f.complete = false;
                    f.install_complete = false;
                }
            _ => {}
        }
    }

    // Propagate sources of linked in-package libraries into executables.
    let lib_sources: HashMap<String, Vec<String>> = targets
        .iter()
        .filter(|t| !t.2)
        .map(|t| (t.0.clone(), t.1.clone()))
        .collect();
    for (name, sources, is_exe, loc) in &targets {
        if !is_exe {
            continue;
        }
        let mut all = sources.clone();
        for (t, libs) in &links {
            if t == name {
                for l in libs {
                    if let Some(s) = lib_sources.get(l) {
                        all.extend(s.iter().cloned());
                    }
                }
            }
        }
        all.sort();
        all.dedup();
        f.executables.push(Executable {
            name: name.clone(),
            sources: all,
            loc: *loc,
            installed: installed.contains(name) || ament_auto,
        });
    }
    if f.install_complete && !ament_auto {
        for (lib, loc) in component_libs {
            let known_target = targets.iter().any(|t| t.0 == lib);
            if known_target
                && !lib.contains('$')
                && !installed_any.contains(&lib)
                && !f.uninstalled_component_libs.iter().any(|(l, _)| *l == lib)
            {
                f.uninstalled_component_libs.push((lib, loc));
            }
        }
    }
    f
}

fn looks_like_source(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    [".cpp", ".cc", ".cxx", ".c", ".hpp", ".h", ".hh", ".cu"]
        .iter()
        .any(|e| l.ends_with(e))
}

#[allow(clippy::too_many_arguments)]
fn interpret_install(
    args: &[String],
    pkg: &str,
    installed: &mut Vec<String>,
    f: &mut CmakeFacts,
    _targets: &mut [(String, Vec<String>, bool, Loc)],
    loc: Loc,
    rel: &dyn Fn(&str) -> String,
) {
    let Some(mode) = args.first() else { return };
    // Collect items until the first keyword.
    let mut items = Vec::new();
    let mut i = 1;
    while i < args.len() && !is_install_keyword(&args[i]) {
        items.push(args[i].clone());
        i += 1;
    }
    let dest = args
        .iter()
        .position(|a| a == "DESTINATION")
        .and_then(|p| args.get(p + 1))
        .map(|d| d.trim_start_matches('/').trim_end_matches('/').to_string())
        .unwrap_or_default();
    let lib_pkg = format!("lib/{pkg}");
    let share_pkg = format!("share/{pkg}");
    match mode.as_str() {
        "TARGETS" => {
            // Executables must land in lib/<pkg> for ros2 run/launch.
            let runtime_dest = args
                .iter()
                .position(|a| a == "RUNTIME")
                .and_then(|p| {
                    args.iter()
                        .skip(p)
                        .position(|a| a == "DESTINATION")
                        .map(|q| p + q)
                })
                .and_then(|p| args.get(p + 1))
                .map(|d| d.trim_end_matches('/').to_string());
            let d = runtime_dest.unwrap_or(dest);
            if d == lib_pkg {
                installed.extend(items);
            }
        }
        "PROGRAMS" => {
            if dest == lib_pkg {
                for it in items {
                    let base = it.rsplit('/').next().unwrap_or(&it).to_string();
                    let _ = rel(&it);
                    f.executables.push(Executable {
                        name: base.clone(),
                        sources: vec![rel(&it)],
                        loc,
                        installed: true,
                    });
                }
            }
        }
        "DIRECTORY" | "FILES" => {
            if dest == share_pkg {
                if mode == "DIRECTORY" {
                    for it in &items {
                        if it.ends_with('/') {
                            // Contents land directly in share/<pkg>: cannot tell which subdirs.
                            f.install_complete = false;
                        } else {
                            f.share_dirs
                                .push(it.rsplit('/').next().unwrap_or(it).to_string());
                        }
                    }
                }
            } else if let Some(sub) = dest.strip_prefix(&format!("{share_pkg}/")) {
                f.share_dirs
                    .push(sub.split('/').next().unwrap_or(sub).to_string());
            }
        }
        _ => {}
    }
}

fn is_install_keyword(a: &str) -> bool {
    matches!(
        a,
        "DESTINATION"
            | "EXPORT"
            | "ARCHIVE"
            | "LIBRARY"
            | "RUNTIME"
            | "INCLUDES"
            | "PERMISSIONS"
            | "COMPONENT"
            | "OPTIONAL"
            | "FILES_MATCHING"
            | "PATTERN"
            | "REGEX"
            | "USE_SOURCE_PERMISSIONS"
            | "RENAME"
            | "CONFIGURATIONS"
            | "EXCLUDE"
            | "TYPE"
            | "MESSAGE_NEVER"
    )
}

/// Expands `${VAR}` references. Returns None if any variable is unknown.
/// A string that is exactly one `${LIST}` expands to the list's items.
fn expand_var(s: &str, vars: &HashMap<String, Vec<String>>) -> Option<Vec<String>> {
    if !s.contains("${") {
        return Some(vec![s.to_string()]);
    }
    if s.starts_with("${")
        && s.ends_with('}')
        && s[2..s.len() - 1]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
    {
        return vars.get(&s[2..s.len() - 1]).cloned();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find("${") {
        out.push_str(&rest[..p]);
        let after = &rest[p + 2..];
        let e = after.find('}')?;
        let v = vars.get(&after[..e])?;
        out.push_str(&v.join(";"));
        rest = &after[e + 1..];
    }
    out.push_str(rest);
    Some(vec![out])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
cmake_minimum_required(VERSION 3.8)
project(demo_pkg)
find_package(rclcpp REQUIRED)
find_package(std_msgs REQUIRED)
set(DEPS rclcpp std_msgs)
add_library(core SHARED src/core.cpp)
add_executable(talker src/talker.cpp)  # comment
target_link_libraries(talker core)
ament_target_dependencies(talker ${DEPS})
add_executable(not_installed src/x.cpp)
install(TARGETS talker DESTINATION lib/${PROJECT_NAME})
install(DIRECTORY launch config DESTINATION share/${PROJECT_NAME})
install(PROGRAMS scripts/tool.py DESTINATION lib/${PROJECT_NAME})
rclcpp_components_register_node(core PLUGIN "demo::Comp" EXECUTABLE comp_node)
ament_package()
"#;

    #[test]
    fn executables_and_installs() {
        let f = interpret(0, SRC, "demo_pkg", "pkgs/demo");
        let exe = |n: &str| f.executables.iter().find(|e| e.name == n).unwrap();
        assert!(exe("talker").installed);
        assert!(exe("talker")
            .sources
            .contains(&"pkgs/demo/src/core.cpp".to_string()));
        assert!(!exe("not_installed").installed);
        assert!(exe("tool.py").installed);
        assert!(exe("comp_node").installed);
        assert_eq!(f.share_dirs, vec!["launch", "config"]);
        assert!(f.uses.iter().any(|u| u.name == "std_msgs"));
        assert!(f.complete);
    }

    #[test]
    fn foreach_marks_incomplete() {
        let f = interpret(
            0,
            "foreach(x a b)\nadd_executable(${x} src/${x}.cpp)\nendforeach()\n",
            "p",
            "",
        );
        assert!(!f.complete);
    }
}
