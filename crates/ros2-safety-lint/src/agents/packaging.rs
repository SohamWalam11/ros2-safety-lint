//! Packaging agent: launch references vs. what packages build, install and declare.

use super::{Agent, Context};
use crate::diag::{Diagnostic, Fix, FixSafety, TextEdit};
use crate::facts::*;
use crate::resolve::{resolve_lval, ImplStatus};
use crate::rules::Severity;
use crate::source::{FileId, Loc};
use crate::util::closest;
use std::collections::{HashMap, HashSet};

pub struct PackagingAgent;

impl Agent for PackagingAgent {
    fn id(&self) -> &'static str {
        "packaging"
    }
    fn title(&self) -> &'static str {
        "Launch & packaging (executables, dependencies, installs, files)"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        super::GRAPH_INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        !cx.ix.packages.is_empty()
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        unregistered_component_classes(cx, out);
        interface_packages(cx, out);
        rosdep_keys(cx, out);
        for p in &cx.ix.packages {
            for (lib, loc) in &p.uninstalled_component_libs {
                out.push(
                    Diagnostic::new("L010", *loc, format!("library `{lib}` of package `{}` registers components but is never installed; containers cannot load it from the install space", p.name))
                        .help(format!("Add `install(TARGETS {lib} ARCHIVE DESTINATION lib LIBRARY DESTINATION lib RUNTIME DESTINATION bin)`.")),
                );
            }
        }
        unknown_executables(cx, out);
        missing_dependencies(cx, out);
        setup_cfg(cx, out);
        uninstalled_dirs(cx, out);
        missing_files(cx, out);
        missing_install_sources(cx, out);
        python_subpackages(cx, out);
    }
}

/// L011: `install(FILES|PROGRAMS|DIRECTORY x …)` where `x` does not exist: the install step
/// of `colcon build` fails ("file INSTALL cannot find").
fn missing_install_sources(cx: &Context, out: &mut Vec<Diagnostic>) {
    for p in &cx.ix.packages {
        let mut seen = HashSet::new();
        for (path, loc) in &p.install_sources {
            if !seen.insert(path.as_str()) || cx.ix.exists_rel(path) || in_empty_dir(cx, path) {
                continue;
            }
            let shown = path.strip_prefix(&format!("{}/", p.dir)).unwrap_or(path);
            out.push(
                Diagnostic::new(
                    "L011",
                    *loc,
                    format!("`{shown}` is installed by package `{}` but does not exist; `colcon build` fails at the install step", p.name),
                )
                .help("Fix the path, create the file, or mark the rule OPTIONAL if the file is generated or platform-specific."),
            );
        }
    }
}

/// A missing path below a directory that exists but is empty: an uninitialised git
/// submodule (or vendored tree) in this checkout, not a broken install rule.
fn in_empty_dir(cx: &Context, rel: &str) -> bool {
    let mut dir = std::path::Path::new(rel).parent();
    while let Some(d) = dir.filter(|d| !d.as_os_str().is_empty()) {
        let abs = cx.ix.root.join(d);
        if abs.is_dir() {
            return std::fs::read_dir(&abs).is_ok_and(|mut it| it.next().is_none());
        }
        dir = d.parent();
    }
    false
}

/// L012: setup.py lists `packages=[...]` by hand and leaves out a subpackage that exists:
/// it is not installed, so importing it from the install space fails.
fn python_subpackages(cx: &Context, out: &mut Vec<Diagnostic>) {
    for p in &cx.ix.packages {
        let Some((listed, loc)) = &p.setup_py_packages else {
            continue;
        };
        let mut missing = Vec::new();
        for top in listed.iter().filter(|t| !t.contains('.')) {
            let rel = if p.dir.is_empty() {
                top.clone()
            } else {
                format!("{}/{top}", p.dir)
            };
            for sub in python_packages_under(&cx.ix.root.join(&rel), top, 0) {
                if !listed.contains(&sub) && !missing.contains(&sub) {
                    missing.push(sub);
                }
            }
        }
        if missing.is_empty() {
            continue;
        }
        let names = missing
            .iter()
            .map(|m| format!("`{m}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push(
            Diagnostic::new(
                "L012",
                *loc,
                format!("setup.py of `{}` lists its Python packages by hand and leaves out {names}; they are not installed, so importing them fails", p.name),
            )
            .help("Use `packages=find_packages(exclude=['test'])`, or add the missing packages to the list."),
        );
    }
}

/// Dotted names of the Python packages (directories with `__init__.py`) below `dir`, whose
/// own dotted name is `name`. Test and cache directories are not packages that get installed.
fn python_packages_under(dir: &std::path::Path, name: &str, depth: usize) -> Vec<String> {
    let mut out = Vec::new();
    if depth > 6 || !dir.join("__init__.py").is_file() {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut subdirs: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
        .filter(|(n, _)| {
            !matches!(n.as_str(), "test" | "tests" | "__pycache__") && !n.starts_with('.')
        })
        .collect();
    subdirs.sort();
    for (sub, path) in subdirs {
        let dotted = format!("{name}.{sub}");
        if path.join("__init__.py").is_file() {
            out.push(dotted.clone());
            out.extend(python_packages_under(&path, &dotted, depth + 1));
        }
    }
    out
}

fn unknown_executables(cx: &Context, out: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for (_, _, n) in cx.graph.instances() {
        if !seen.insert(n.decl.loc) || cx.ix.external_files.contains(&n.decl.loc.file) {
            continue;
        }
        if n.decl.kind == LaunchNodeKind::Composable {
            // Components have a plugin, not an executable.
            let Some(pkg) = &n.pkg else { continue };
            let Some(p) = cx.ix.package_by_name(pkg) else {
                continue;
            };
            let Some(plugin) = n
                .decl
                .plugin
                .as_ref()
                .and_then(|x| resolve_lval(x, &Default::default()))
            else {
                continue;
            };
            // A package whose CMake never calls rclcpp_components_register_node(s) registers
            // no components at all (and its registrations cannot hide in unresolved variables).
            let registers_none = p.plugin_classes.is_empty()
                && p.cmake_file
                    .is_some_and(|f| !cx.ix.file(f).text.contains("rclcpp_components_register"));
            if p.executables_complete
                && (!p.plugin_classes.is_empty() || registers_none)
                && !p.plugin_classes.contains(&plugin)
            {
                let mut d = Diagnostic::new(
                    "L001",
                    n.decl.loc,
                    if registers_none {
                        format!("component `{plugin}` is loaded, but package `{pkg}` registers no components (no `rclcpp_components_register_nodes` in its CMakeLists.txt); the container cannot load it")
                    } else {
                        format!("component `{plugin}` is not registered by package `{pkg}`")
                    },
                );
                if let Some(c) = closest(&plugin, p.plugin_classes.iter().map(|s| s.as_str())) {
                    d = d.help(format!("Did you mean `{c}`?"));
                }
                out.push(d);
            }
            continue;
        }
        let (Some(pkg), Some(exec)) = (&n.pkg, &n.exec) else {
            continue;
        };
        if n.impl_status == ImplStatus::External {
            installed_executable(cx, n, pkg, exec, out);
            continue;
        }
        let Some(p) = cx.ix.package_by_name(pkg) else {
            continue;
        };
        if !p.executables_complete {
            continue;
        }
        match p.executables.iter().find(|e| e.name == *exec) {
            None => {
                let mut d = Diagnostic::new(
                    "L001",
                    n.decl.loc,
                    format!("package `{pkg}` has no executable `{exec}`"),
                );
                if let Some(span) = super::value_span(cx.ix, n.decl.loc, exec) {
                    d = d.repair(
                        span,
                        exec.as_str(),
                        p.executables
                            .iter()
                            .filter(|e| e.installed)
                            .map(|e| e.name.clone())
                            .collect(),
                    );
                }
                if let Some(c) = closest(exec, p.executables.iter().map(|e| e.name.as_str())) {
                    d = d.help(format!("Did you mean `{c}`?"));
                } else if !p.executables.is_empty() {
                    let names: Vec<&str> = p
                        .executables
                        .iter()
                        .map(|e| e.name.as_str())
                        .take(8)
                        .collect();
                    d = d.help(format!("Executables in `{pkg}`: {}", names.join(", ")));
                }
                out.push(d);
            }
            Some(e)
                if !e.installed
                    && p.install_rules_complete
                    && p.build_type != BuildType::AmentPython =>
            {
                out.push(
                    Diagnostic::new("L001", n.decl.loc, format!("`{exec}` is built by `{pkg}` but never installed into `lib/{pkg}`, so it cannot be launched"))
                        .related(e.loc, "target defined here")
                        .help(format!("Add `install(TARGETS {exec} DESTINATION lib/${{PROJECT_NAME}})`.")),
                );
            }
            _ => {}
        }
    }
}

/// A launched package that is not in the workspace: check it against the installed ROS.
fn installed_executable(
    cx: &Context,
    n: &crate::resolve::NodeInstance,
    pkg: &str,
    exec: &str,
    out: &mut Vec<Diagnostic>,
) {
    let env = &cx.ix.installed;
    if env.is_empty() {
        return; // no ROS installation visible: nothing to compare against
    }
    let Some(ip) = env.get(pkg) else {
        let mut d = Diagnostic::new(
            "L001",
            n.decl.loc,
            format!("package `{pkg}` is neither in this workspace nor installed in the ROS environment"),
        )
        .severity(Severity::Warning)
        .help("Install it (e.g. `rosdep install --from-paths src -y --ignore-src`) or fix the package name.");
        if let Some(c) = closest(pkg, env.packages.keys().map(|s| s.as_str())) {
            d = d.help(format!("Did you mean the installed package `{c}`?"));
        }
        out.push(d);
        return;
    };
    match n.decl.kind {
        LaunchNodeKind::Composable => {
            let Some(plugin) = n
                .decl
                .plugin
                .as_ref()
                .and_then(|x| resolve_lval(x, &Default::default()))
            else {
                return;
            };
            if !ip.components.is_empty() && !ip.components.contains(&plugin) {
                let mut d = Diagnostic::new(
                    "L001",
                    n.decl.loc,
                    format!("installed package `{pkg}` registers no component `{plugin}`"),
                );
                if let Some(c) = closest(&plugin, ip.components.iter().map(|s| s.as_str())) {
                    d = d.help(format!("Did you mean `{c}`?"));
                }
                out.push(d);
            }
        }
        _ => {
            if !ip.executables.is_empty() && !ip.executables.iter().any(|e| e == exec) {
                let mut d = Diagnostic::new(
                    "L001",
                    n.decl.loc,
                    format!("installed package `{pkg}` has no executable `{exec}`"),
                );
                if let Some(span) = super::value_span(cx.ix, n.decl.loc, exec) {
                    d = d.repair(span, exec, ip.executables.clone());
                }
                if let Some(c) = closest(exec, ip.executables.iter().map(|s| s.as_str())) {
                    d = d.help(format!("Did you mean `{c}`?"));
                } else {
                    d = d.help(format!(
                        "Executables in `{pkg}`: {}",
                        ip.executables
                            .iter()
                            .take(8)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                out.push(d);
            }
        }
    }
}

/// Common ROS packages, so that `find_package`/`import` checks ignore system libraries.
fn looks_ros(name: &str, workspace: &HashSet<&str>) -> bool {
    const KNOWN: &[&str] = &[
        "rclcpp",
        "rclcpp_components",
        "rclcpp_action",
        "rclcpp_lifecycle",
        "rclpy",
        "rcl",
        "rcutils",
        "rmw",
        "tf2",
        "tf2_ros",
        "tf2_ros_py",
        "tf2_geometry_msgs",
        "tf2_eigen",
        "tf2_sensor_msgs",
        "tf2_kdl",
        "launch",
        "launch_ros",
        "ament_index_cpp",
        "ament_index_python",
        "pluginlib",
        "class_loader",
        "message_filters",
        "image_transport",
        "cv_bridge",
        "pcl_conversions",
        "pcl_ros",
        "laser_geometry",
        "angles",
        "urdf",
        "kdl_parser",
        "robot_state_publisher",
        "controller_interface",
        "hardware_interface",
        "controller_manager",
        "realtime_tools",
        "control_toolbox",
        "generate_parameter_library",
        "nav2_util",
        "nav2_core",
        "nav2_costmap_2d",
        "nav2_common",
        "behaviortree_cpp",
        "behaviortree_cpp_v3",
        "rosidl_default_generators",
        "rosidl_default_runtime",
        "builtin_interfaces",
        "xacro",
        "rviz2",
        "rviz_common",
        "rviz_default_plugins",
        "moveit_core",
        "moveit_ros_planning_interface",
    ];
    workspace.contains(name)
        || KNOWN.contains(&name)
        || ["_msgs", "_srvs", "_interfaces", "_action", "_actions"]
            .iter()
            .any(|s| name.ends_with(s))
}

fn dep_kinds_ok(kind: UseKind, d: DepKind) -> bool {
    match kind {
        UseKind::CmakeFindPackage => !matches!(d, DepKind::Exec),
        UseKind::PythonImport | UseKind::LaunchReference | UseKind::LaunchImport => {
            matches!(d, DepKind::Depend | DepKind::Exec | DepKind::Test)
        }
    }
}

fn missing_dependencies(cx: &Context, out: &mut Vec<Diagnostic>) {
    let workspace: HashSet<&str> = cx.ix.packages.iter().map(|p| p.name.as_str()).collect();
    // Launch references, collected statically from each package's launch files.
    let mut launch_uses: HashMap<usize, Vec<PackageUse>> = HashMap::new();
    for (fid, lf) in &cx.ix.launches {
        let Some(p) = cx.ix.file_pkg.get(*fid).copied().flatten() else {
            continue;
        };
        let mut v = Vec::new();
        collect_launch_refs(&lf.items, &mut v);
        launch_uses.entry(p).or_default().extend(v);
    }
    for (pi, p) in cx.ix.packages.iter().enumerate() {
        let mut reported = HashSet::new();
        let via_meta = metapackage_closure(cx, p);
        let uses = p
            .uses
            .iter()
            .cloned()
            .chain(launch_uses.remove(&pi).unwrap_or_default());
        for u in uses {
            // Anything a launch file starts or includes is a ROS package by definition; for
            // find_package/import the heuristic keeps system libraries (Eigen, numpy, …) out.
            let ros = u.kind == UseKind::LaunchReference || looks_ros(&u.name, &workspace);
            if u.name == p.name
                || u.name.is_empty()
                || !ros
                || u.name.starts_with("ament_cmake")
                || u.name.starts_with("ament_lint")
            {
                continue;
            }
            if p.deps
                .iter()
                .any(|d| d.name == u.name && dep_kinds_ok(u.kind, d.kind))
            {
                continue;
            }
            // Runtime uses satisfied through a workspace metapackage (e.g. nav2_bringup → navigation2).
            if u.kind != UseKind::CmakeFindPackage && via_meta.contains(u.name.as_str()) {
                continue;
            }
            if !reported.insert(u.name.clone()) {
                continue;
            }
            let (tag, why) = match u.kind {
                UseKind::CmakeFindPackage => (
                    "depend",
                    "found with find_package()/ament_target_dependencies()",
                ),
                UseKind::PythonImport => ("depend", "imported by Python code"),
                UseKind::LaunchReference => ("exec_depend", "used by a launch file"),
                UseKind::LaunchImport => ("exec_depend", "imported by a launch file"),
            };
            let mut d = Diagnostic::new(
                "L002",
                u.loc,
                format!(
                    "`{}` is {why} in package `{}` but not declared in its package.xml",
                    u.name, p.name
                ),
            );
            if let Some(f) = insert_dependency(cx, p, tag, &u.name) {
                d = d.fix(f);
            }
            if u.kind == UseKind::LaunchImport {
                d = d.severity(Severity::Info);
            }
            out.push(d);
        }
    }
}

/// Packages reachable through exec/depend edges of workspace packages this package depends on.
fn metapackage_closure<'a>(cx: &'a Context, p: &'a Package) -> HashSet<&'a str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = p
        .deps
        .iter()
        .filter(|d| matches!(d.kind, DepKind::Exec | DepKind::Depend))
        .map(|d| d.name.as_str())
        .collect();
    while let Some(n) = stack.pop() {
        if !seen.insert(n) {
            continue;
        }
        if let Some(q) = cx.ix.package_by_name(n) {
            stack.extend(
                q.deps
                    .iter()
                    .filter(|d| matches!(d.kind, DepKind::Exec | DepKind::Depend))
                    .map(|d| d.name.as_str()),
            );
        }
    }
    seen
}

fn collect_launch_refs(items: &[LaunchItem], out: &mut Vec<PackageUse>) {
    fn shares(v: &LVal, loc: Loc, out: &mut Vec<PackageUse>) {
        match v {
            LVal::PkgShare(p) | LVal::PkgPrefix(p) => out.push(PackageUse {
                name: p.clone(),
                kind: UseKind::LaunchReference,
                loc,
            }),
            LVal::Concat(xs) | LVal::PathJoin(xs) => xs.iter().for_each(|x| shares(x, loc, out)),
            _ => {}
        }
    }
    for it in items {
        match it {
            LaunchItem::Node(n) => {
                if let LVal::Lit(p) = &n.pkg {
                    out.push(PackageUse {
                        name: p.clone(),
                        kind: UseKind::LaunchReference,
                        loc: n.loc,
                    });
                }
                for s in &n.params {
                    if let LaunchParamSource::File { path, loc, .. } = s {
                        shares(path, *loc, out);
                    }
                }
            }
            LaunchItem::Include { path, loc, .. } => shares(path, *loc, out),
            LaunchItem::Group { items, .. } | LaunchItem::Cond { items, .. } => {
                collect_launch_refs(items, out)
            }
            _ => {}
        }
    }
}

/// Inserts `<tag>name</tag>` after the last dependency element of package.xml (or after
/// `<name>`): on a new line with the same indentation when that element ends its line,
/// otherwise directly after the element, so it always stays inside `<package>`.
fn insert_dependency(cx: &Context, p: &Package, tag: &str, name: &str) -> Option<Fix> {
    let f = cx.ix.file(p.manifest);
    let anchor = p
        .deps
        .iter()
        .map(|d| d.loc.end)
        .max()
        .unwrap_or(p.manifest_name_loc.end);
    let rest_of_line = f.text[anchor..].split('\n').next().unwrap_or("");
    let element = format!("<{tag}>{name}</{tag}>");
    let (start, replacement) = if rest_of_line.trim().is_empty() && f.text[anchor..].contains('\n')
    {
        let line_end = anchor + rest_of_line.len() + 1;
        let (line, _) = f.line_col(anchor);
        let indent: String = f
            .line_text(line)
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        let eol = if rest_of_line.ends_with('\r') {
            "\r\n"
        } else {
            "\n"
        };
        (line_end, format!("{indent}{element}{eol}"))
    } else {
        (anchor, element.clone())
    };
    Some(Fix {
        description: format!("Add `{element}` to package.xml"),
        safety: FixSafety::Safe,
        edits: vec![TextEdit {
            file: p.manifest,
            start,
            end: start,
            replacement,
        }],
    })
}

fn setup_cfg(cx: &Context, out: &mut Vec<Diagnostic>) {
    for p in &cx.ix.packages {
        let Some(py) = &p.python else { continue };
        if p.build_type != BuildType::AmentPython
            || !py.has_console_scripts
            || py.setup_cfg_install_scripts_ok
        {
            continue;
        }
        let block = format!(
            "[develop]\nscript_dir=$base/lib/{0}\n[install]\ninstall_scripts=$base/lib/{0}\n",
            p.name
        );
        match py.setup_cfg {
            Some(cfg) => {
                let f = cx.ix.file(cfg);
                let has_install = f
                    .text
                    .lines()
                    .any(|l| l.trim().eq_ignore_ascii_case("[install]"));
                let mut d = Diagnostic::new(
                    "L003",
                    Loc::new(cfg, 0, 0),
                    format!("setup.cfg of `{}` does not install console scripts into `lib/{}`; `ros2 run`/`ros2 launch` will not find them", p.name, p.name),
                );
                if !has_install {
                    let end = f.text.len();
                    let sep = if f.text.ends_with('\n') || f.text.is_empty() {
                        ""
                    } else {
                        "\n"
                    };
                    d = d.safe_fix(
                        "Add the [develop]/[install] script directories",
                        cfg,
                        end,
                        end,
                        format!("{sep}{block}"),
                    );
                } else {
                    d = d.help(format!(
                        "Set `install_scripts=$base/lib/{}` in the [install] section.",
                        p.name
                    ));
                }
                out.push(d);
            }
            None => {
                let loc = py.console_scripts_loc.unwrap_or(Loc::new(p.manifest, 0, 0));
                out.push(
                    Diagnostic::new("L003", loc, format!("package `{}` has console scripts but no setup.cfg; they will be installed to `bin/` where ros2 cannot find them", p.name))
                        .help(format!("Create `{}/setup.cfg` containing:\n{block}", if p.dir.is_empty() { "." } else { &p.dir })),
                );
            }
        }
    }
}

/// L009: package.xml dependencies `rosdep install` cannot resolve: neither a package released
/// in the distribution, a rosdep system key, nor a package of this workspace. Only with a known
/// distribution. A near-miss of a real key is a warning (typo); anything else is info, since it
/// may come from another workspace or a `--skip-keys` list.
fn rosdep_keys(cx: &Context, out: &mut Vec<Diagnostic>) {
    let Some(distro) = cx.ix.installed.distro.as_deref() else {
        return;
    };
    let Some(keys) = crate::pack::rosdep_keys(distro) else {
        return;
    };
    let workspace: HashSet<&str> = cx.ix.packages.iter().map(|p| p.name.as_str()).collect();
    for p in &cx.ix.packages {
        let mut seen = HashSet::new();
        for d in &p.deps {
            let n = d.name.as_str();
            if n.is_empty()
                || n.contains('$')
                || workspace.contains(n)
                || keys.contains(n)
                || !seen.insert(n)
            {
                continue;
            }
            let near = closest(n, keys.iter().map(String::as_str));
            let mut diag = Diagnostic::new(
                "L009",
                d.loc,
                format!("`{n}` (package `{}`) is not a rosdep key or a released package of this ROS distribution, nor a workspace package; `rosdep install` cannot resolve it", p.name),
            );
            match near {
                Some(c) => diag = diag.help(format!("Did you mean `{c}`?")),
                None => {
                    diag = diag
                        .severity(Severity::Info)
                        .help("If it comes from another workspace, add it there or pass it to `rosdep install --skip-keys`.")
                }
            }
            out.push(diag);
        }
    }
}

/// L007: interface files and `rosidl_generate_interfaces` disagree.
/// L008: a package generates interfaces but is not in the `rosidl_interface_packages` group.
fn interface_packages(cx: &Context, out: &mut Vec<Diagnostic>) {
    for p in &cx.ix.packages {
        if p.build_type != BuildType::AmentCmake {
            continue;
        }
        let prefix = if p.dir.is_empty() {
            String::new()
        } else {
            format!("{}/", p.dir)
        };
        // Interface files of this package in its msg/, srv/ and action/ folders.
        let own: Vec<String> = cx
            .ix
            .interface_files
            .iter()
            .filter_map(|f| f.strip_prefix(&prefix))
            .filter(|r| {
                matches!(r.split('/').next(), Some("msg" | "srv" | "action"))
                    && r.matches('/').count() == 1
            })
            .map(str::to_string)
            .collect();
        let cmake_text = p
            .cmake_file
            .map(|f| cx.ix.file(f).text.as_str())
            .unwrap_or_default();
        let Some(call) = &p.rosidl else {
            // Interface files but no generation at all (ROS 1 and included-cmake setups excluded).
            if !own.is_empty()
                && p.executables_complete
                && p.cmake_file.is_some()
                && !cmake_text.contains("rosidl_generate_interfaces")
                && !cmake_text.contains("add_message_files")
                && !cmake_text.contains("include(")
            {
                out.push(
                    Diagnostic::new(
                        "L007",
                        p.manifest_name_loc,
                        format!("package `{}` has interface files ({}) but never calls `rosidl_generate_interfaces`; no code is generated for them", p.name, own.join(", ")),
                    )
                    .help("Add `find_package(rosidl_default_generators REQUIRED)` and `rosidl_generate_interfaces(${PROJECT_NAME} …)` to CMakeLists.txt."),
                );
            }
            continue;
        };
        let manifest = &cx.ix.file(p.manifest).text;
        if !call.skip_group_check && !manifest.contains("rosidl_interface_packages") {
            out.push(
                Diagnostic::new(
                    "L008",
                    p.manifest_name_loc,
                    format!("package `{}` generates interfaces but its package.xml does not declare `<member_of_group>rosidl_interface_packages</member_of_group>`; CMake configuration fails", p.name),
                )
                .help("Add `<member_of_group>rosidl_interface_packages</member_of_group>` (and `rosidl_default_generators` / `rosidl_default_runtime` dependencies) to package.xml."),
            );
        }
        for f in call
            .files
            .iter()
            .filter(|f| [".msg", ".srv", ".action"].iter().any(|e| f.ends_with(e)))
        {
            if !own.contains(f)
                && !cx
                    .ix
                    .interface_files
                    .iter()
                    .any(|x| *x == format!("{prefix}{f}"))
            {
                out.push(
                    Diagnostic::new("L007", call.loc, format!("`rosidl_generate_interfaces` lists `{f}`, which does not exist in package `{}`; CMake configuration fails", p.name))
                        .severity(Severity::Error),
                );
            }
        }
        if call.complete {
            for f in own.iter().filter(|f| !call.files.contains(f)) {
                out.push(Diagnostic::new(
                    "L007",
                    call.loc,
                    format!("`{f}` is not listed in `rosidl_generate_interfaces`, so no code is generated for it; users of the type fail to build"),
                ));
            }
        }
    }
}

/// L006: a class registered as a component in CMake whose sources never use
/// `RCLCPP_COMPONENTS_REGISTER_NODE(<class>)`: the class loader finds no factory for it.
fn unregistered_component_classes(cx: &Context, out: &mut Vec<Diagnostic>) {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"RCLCPP_COMPONENTS_REGISTER_NODE\s*\(\s*(?:::)?([A-Za-z_][\w:]*)\s*\)")
            .expect("valid regex")
    });
    for (pi, p) in cx.ix.packages.iter().enumerate() {
        let Some(cm) = p.cmake_file else { continue };
        let registered: Vec<&String> = p
            .plugin_classes
            .iter()
            .filter(|c| c.contains("::") && !c.contains('$'))
            .collect();
        if registered.is_empty() || !cx.ix.file(cm).text.contains("rclcpp_components_register") {
            continue; // (pluginlib-only packages register plugins, not components)
        }
        let macros: HashSet<String> = cx
            .ix
            .code
            .keys()
            .filter(|f| cx.ix.file_pkg.get(**f).copied().flatten() == Some(pi))
            .flat_map(|f| {
                re.captures_iter(&cx.ix.file(*f).text)
                    .map(|c| c[1].to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
        let cm_text = &cx.ix.file(cm).text;
        for cls in registered {
            if macros.contains(cls.as_str()) || !cm_text.contains(cls.as_str()) {
                continue; // registered via pluginlib XML, not CMake: not ours to check
            }
            let start = cm_text.find(cls.as_str()).unwrap_or(0);
            let mut d = Diagnostic::new(
                "L006",
                Loc::new(cm, start, start + cls.len()),
                format!("`{cls}` is registered as a component, but no source file of package `{}` contains `RCLCPP_COMPONENTS_REGISTER_NODE({cls})`; loading it fails", p.name),
            )
            .help("Add `#include <rclcpp_components/register_node_macro.hpp>` and `RCLCPP_COMPONENTS_REGISTER_NODE(…)` to the class's source file.");
            if let Some(c) = closest(cls, macros.iter().map(String::as_str)) {
                d = d.help(format!("The sources register `{c}`; did you mean that?"));
            }
            out.push(d);
        }
    }
}

fn uninstalled_dirs(cx: &Context, out: &mut Vec<Diagnostic>) {
    // dir references per package: "launch" whenever the package has launch files, plus share paths used by launch files.
    let mut wanted: HashMap<usize, HashMap<String, Loc>> = HashMap::new();
    // Test launch files and ROS 1 launch files (`.launch`, `.test`) are not run from the
    // install space, so they do not need anything installed.
    let runtime = |fid: FileId| {
        let rel = &cx.ix.file(fid).rel_path;
        !(cx.config.is_test_path(rel) || rel.ends_with(".launch") || rel.ends_with(".test"))
    };
    for fid in cx.ix.launches.keys().copied().filter(|f| runtime(*f)) {
        let fid = &fid;
        let Some(pi) = cx.ix.file_pkg.get(*fid).copied().flatten() else {
            continue;
        };
        let p = &cx.ix.packages[pi];
        let rel = &cx.ix.file(*fid).rel_path;
        let inner = rel.strip_prefix(&format!("{}/", p.dir)).unwrap_or(rel);
        if let Some((top, _)) = inner.split_once('/') {
            wanted
                .entry(pi)
                .or_default()
                .entry(top.to_string())
                .or_insert(Loc::new(*fid, 0, 0));
        }
    }
    for root in cx.graph.roots.iter().filter(|r| runtime(r.root)) {
        for n in &root.nodes {
            for s in &n.decl.params {
                if let LaunchParamSource::File { path, loc, .. } = s {
                    if let Some(r) = resolve_lval(path, &Default::default()) {
                        if let Some(rest) = r.strip_prefix("@share:") {
                            if let Some((pkg, sub)) = rest.split_once('/') {
                                if let (Some(pi), Some((top, _))) = (
                                    cx.ix.packages.iter().position(|p| p.name == pkg),
                                    sub.split_once('/'),
                                ) {
                                    wanted
                                        .entry(pi)
                                        .or_default()
                                        .entry(top.to_string())
                                        .or_insert(*loc);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut keys: Vec<_> = wanted.into_iter().collect();
    keys.sort_by_key(|(k, _)| *k);
    for (pi, dirs) in keys {
        let p = &cx.ix.packages[pi];
        if !p.install_rules_complete || p.installed_share_dirs.iter().any(|d| d == ".") {
            continue;
        }
        let build_file: Option<FileId> =
            p.cmake_file.or(p.python.as_ref().and_then(|x| x.setup_py));
        let Some(bf) = build_file else { continue };
        let mut dirs: Vec<_> = dirs.into_iter().collect();
        dirs.sort();
        for (dir, used_at) in dirs {
            if p.installed_share_dirs.iter().any(|d| d == &dir)
                || !cx.ix.exists_rel(&if p.dir.is_empty() {
                    dir.clone()
                } else {
                    format!("{}/{dir}", p.dir)
                })
            {
                continue;
            }
            let mut d = Diagnostic::new(
                "L004",
                Loc::new(bf, 0, 0),
                format!(
                    "`{dir}/` of package `{}` is used at runtime but never installed into share/{}",
                    p.name, p.name
                ),
            )
            .related(used_at, format!("`{dir}/` used here"));
            if p.cmake_file == Some(bf) {
                let f = cx.ix.file(bf);
                if let Some(pos) = f
                    .text
                    .find("ament_package(")
                    .or_else(|| f.text.find("ament_auto_package("))
                {
                    if !f.text[pos..].starts_with("ament_auto") {
                        d = d.safe_fix(
                            format!("Install `{dir}/` before ament_package()"),
                            bf,
                            pos,
                            pos,
                            format!(
                                "install(DIRECTORY {dir} DESTINATION share/${{PROJECT_NAME}})\n\n"
                            ),
                        );
                    }
                }
            } else {
                d = d.help(format!("Add `(os.path.join('share', package_name, '{dir}'), glob('{dir}/*'))` to data_files in setup.py."));
            }
            out.push(d);
        }
    }
}

fn missing_files(cx: &Context, out: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for root in &cx.graph.roots {
        for (rel, loc) in &root.missing_files {
            if seen.insert((rel.clone(), *loc)) {
                let fname = rel.rsplit('/').next().unwrap_or(rel);
                let dir = rel.rsplit_once('/').map_or("", |x| x.0);
                let siblings: Vec<String> = cx
                    .ix
                    .files_under(dir)
                    .map(|i| {
                        cx.ix
                            .file(i)
                            .rel_path
                            .rsplit('/')
                            .next()
                            .unwrap_or("")
                            .to_string()
                    })
                    .collect();
                let mut d = Diagnostic::new("L005", *loc, format!("`{rel}` does not exist"));
                if let Some(c) = closest(fname, siblings.iter().map(|s| s.as_str())) {
                    d = d.help(format!("Did you mean `{c}`?"));
                }
                out.push(d);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn packaging_checks() {
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\">\n  <name>p</name>\n  <depend>rclcpp</depend>\n</package>\n"),
            ("p/CMakeLists.txt", "project(p)\nfind_package(rclcpp REQUIRED)\nfind_package(sensor_msgs REQUIRED)\nadd_executable(talker src/t.cpp)\nadd_executable(hidden src/h.cpp)\ninstall(TARGETS talker DESTINATION lib/${PROJECT_NAME})\nament_package()\n"),
            ("p/launch/a.launch.xml", "<launch><node pkg=\"p\" exec=\"talkr\"/><node pkg=\"p\" exec=\"hidden\"/><node pkg=\"p\" exec=\"talker\"><param from=\"$(find-pkg-share p)/config/missing.yaml\"/></node></launch>"),
            ("q/package.xml", "<package format=\"3\"><name>q</name><export><build_type>ament_python</build_type></export></package>"),
            ("q/setup.py", "from setuptools import setup\nsetup(name='q', entry_points={'console_scripts': ['n = q.n:main']})\n"),
            ("q/setup.cfg", "[metadata]\nname=q\n"),
        ]);
        let ids: Vec<&str> = d.iter().map(|x| x.rule).collect();
        assert_eq!(ids.iter().filter(|r| **r == "L001").count(), 2, "{d:#?}");
        assert!(
            d.iter()
                .any(|x| x.rule == "L002" && x.message.contains("sensor_msgs")),
            "{d:#?}"
        );
        assert!(ids.contains(&"L003"), "{d:#?}");
        assert!(ids.contains(&"L004"), "{d:#?}");
        assert!(ids.contains(&"L005"), "{d:#?}");
        let l002 = d.iter().find(|x| x.rule == "L002").unwrap();
        assert_eq!(
            l002.fix.as_ref().unwrap().edits[0].replacement,
            "  <depend>sensor_msgs</depend>\n"
        );
    }

    /// ROS 1 launch files inside `test/` (moveit2 chomp) do not need `test/` installed.
    #[test]
    fn test_and_ros1_launch_files_need_nothing_installed() {
        let d = analyze_files(&[
            (
                "p/package.xml",
                "<package format=\"3\"><name>p</name></package>",
            ),
            (
                "p/CMakeLists.txt",
                "project(p)
ament_package()
",
            ),
            (
                "p/test/rrbot.launch",
                "<launch><node pkg=\"p\" exec=\"x\"/></launch>",
            ),
            (
                "p/test/chomp.test",
                "<launch><include file=\"$(find p)/test/rrbot.launch\"/></launch>",
            ),
        ]);
        assert!(d.iter().all(|x| x.rule != "L004"), "{d:#?}");
    }

    /// Components: loaded from a package that registers none (L001), registered in CMake
    /// without `RCLCPP_COMPONENTS_REGISTER_NODE` (L006), and a correct package (silent).
    #[test]
    fn component_registration() {
        let launch = |pkg: &str| {
            format!("from launch import LaunchDescription\nfrom launch_ros.actions import ComposableNodeContainer\nfrom launch_ros.descriptions import ComposableNode\ndef generate_launch_description():\n    return LaunchDescription([ComposableNodeContainer(name='c', namespace='', package='rclcpp_components', executable='component_container', composable_node_descriptions=[ComposableNode(package='{pkg}', plugin='{pkg}::Talker', name='talker')])])\n")
        };
        let pkg = |name: &str, cmake: &str, cpp: &str, l: &str| {
            vec![
                (
                    format!("{name}/package.xml"),
                    format!("<package format=\"3\"><name>{name}</name></package>"),
                ),
                (format!("{name}/CMakeLists.txt"), cmake.to_string()),
                (format!("{name}/src/talker.cpp"), cpp.to_string()),
                (format!("{name}/launch/a.launch.py"), l.to_string()),
            ]
        };
        let cpp = |ns: &str, with_macro: bool| {
            format!("#include <rclcpp/rclcpp.hpp>\nnamespace {ns} {{ class Talker : public rclcpp::Node {{ public: explicit Talker(const rclcpp::NodeOptions & o) : Node(\"talker\", o) {{}} }}; }}\n{}", if with_macro { format!("#include <rclcpp_components/register_node_macro.hpp>\nRCLCPP_COMPONENTS_REGISTER_NODE({ns}::Talker)\n") } else { String::new() })
        };
        let mut files = Vec::new();
        // a: no registration at all
        files.extend(pkg("a", "project(a)\nadd_library(a SHARED src/talker.cpp)\ninstall(TARGETS a DESTINATION lib)\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n", &cpp("a", true), &launch("a")));
        // b: CMake registration, no macro
        files.extend(pkg("b", "project(b)\nadd_library(b SHARED src/talker.cpp)\nrclcpp_components_register_nodes(b \"b::Talker\")\ninstall(TARGETS b DESTINATION lib)\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n", &cpp("b", false), &launch("b")));
        // c: correct
        files.extend(pkg("c", "project(c)\nadd_library(c SHARED src/talker.cpp)\nrclcpp_components_register_nodes(c \"c::Talker\")\ninstall(TARGETS c DESTINATION lib)\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n", &cpp("c", true), &launch("c")));
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let d = analyze_files(&refs);
        let by = |rule: &str, pkg: &str| {
            d.iter()
                .filter(|x| x.rule == rule && x.message.contains(&format!("{pkg}::Talker")))
                .count()
        };
        assert_eq!(by("L001", "a"), 1, "{d:#?}");
        assert_eq!(by("L006", "b"), 1, "{d:#?}");
        assert_eq!(
            by("L001", "c") + by("L006", "c") + by("L001", "b") + by("L006", "a"),
            0,
            "{d:#?}"
        );
    }

    /// Interface packages: unlisted file and missing group (L007, L008); a correct one is silent.
    #[test]
    fn interface_packages() {
        let pkg = |name: &str, listed: &str, group: bool| {
            vec![
                (format!("{name}/package.xml"), format!("<package format=\"3\"><name>{name}</name>{}</package>", if group { "<member_of_group>rosidl_interface_packages</member_of_group>" } else { "" })),
                (format!("{name}/CMakeLists.txt"), format!("project({name})\nfind_package(rosidl_default_generators REQUIRED)\nrosidl_generate_interfaces(${{PROJECT_NAME}} {listed} DEPENDENCIES std_msgs)\nament_package()\n")),
                (format!("{name}/msg/A.msg"), "int32 a\n".to_string()),
                (format!("{name}/msg/B.msg"), "int32 b\n".to_string()),
            ]
        };
        let mut f = pkg("bad", "\"msg/A.msg\" \"msg/C.msg\"", false);
        f.extend(pkg("good", "\"msg/A.msg\" \"msg/B.msg\"", true));
        let refs: Vec<(&str, &str)> = f.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let d = analyze_files(&refs);
        let msgs: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "L007" || x.rule == "L008")
            .map(|x| x.message.as_str())
            .collect();
        assert!(
            msgs.iter().any(|m| m.contains("`msg/B.msg` is not listed")),
            "{msgs:#?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("lists `msg/C.msg`")),
            "{msgs:#?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("package `bad` generates interfaces")),
            "{msgs:#?}"
        );
        assert!(msgs.iter().all(|m| !m.contains("good")), "{msgs:#?}");
    }

    /// L009 (needs a distribution) and L010 on one workspace.
    #[test]
    fn rosdep_keys_and_uninstalled_component_library() {
        let dir = tempfile::tempdir().unwrap();
        let w = |r: &str, t: &str| {
            let p = dir.path().join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w("c/package.xml", "<package format=\"3\"><name>c</name><depend>rclcpp</depend><depend>rclcpp_componets</depend><depend>my_private_driver</depend></package>");
        w("c/CMakeLists.txt", "project(c)\nadd_library(talker SHARED src/t.cpp)\nadd_library(listener SHARED src/l.cpp)\nrclcpp_components_register_nodes(talker \"c::Talker\")\nrclcpp_components_register_nodes(listener \"c::Listener\")\ninstall(TARGETS listener ARCHIVE DESTINATION lib LIBRARY DESTINATION lib)\n");
        let opts = crate::engine::Options {
            distro: Some("jazzy".into()),
            ..Default::default()
        };
        let d = crate::engine::analyze(dir.path(), &crate::config::Config::default(), &opts)
            .diagnostics;
        let l009: Vec<_> = d.iter().filter(|x| x.rule == "L009").collect();
        assert_eq!(l009.len(), 2, "{l009:#?}");
        assert!(
            l009.iter().any(|x| x.message.contains("rclcpp_componets")
                && x.severity == crate::rules::Severity::Warning
                && x.help
                    .as_deref()
                    .is_some_and(|h| h.contains("rclcpp_components"))),
            "{l009:#?}"
        );
        assert!(
            l009.iter().any(|x| x.message.contains("my_private_driver")
                && x.severity == crate::rules::Severity::Info),
            "{l009:#?}"
        );
        let l010: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "L010")
            .map(|x| x.message.as_str())
            .collect();
        assert!(l010.len() == 1 && l010[0].contains("`talker`"), "{l010:#?}");
        // Without a known distribution, rosdep keys are not checked.
        let none = crate::engine::analyze(
            dir.path(),
            &crate::config::Config::default(),
            &Default::default(),
        )
        .diagnostics;
        assert!(none.iter().all(|x| x.rule != "L009"), "{none:#?}");
    }

    #[test]
    fn install_sources_must_exist() {
        let cmake = "cmake_minimum_required(VERSION 3.8)\nproject(p)\nfind_package(ament_cmake REQUIRED)\ninstall(FILES config/params.yaml config/gone.yaml DESTINATION share/${PROJECT_NAME}/config)\ninstall(PROGRAMS scripts/tool.py DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch maps DESTINATION share/${PROJECT_NAME})\ninstall(FILES include/p/a.h TYPE INCLUDE)\ninstall(FILES win/extra.dll DESTINATION bin OPTIONAL)\ninstall(FILES ${CMAKE_CURRENT_BINARY_DIR}/generated.yaml DESTINATION share/${PROJECT_NAME})\ninstall(FILES ${UNKNOWN_DIR}/x.yaml DESTINATION share/${PROJECT_NAME})\nament_package()\n";
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name><export><build_type>ament_cmake</build_type></export></package>"),
            ("p/CMakeLists.txt", cmake),
            ("p/config/params.yaml", "p:\n  ros__parameters:\n    a: 1\n"),
            ("p/launch/a.launch.xml", "<launch/>"),
            ("p/include/p/a.h", "#pragma once\n"),
        ]);
        let mut l011: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "L011")
            .map(|x| x.message.as_str())
            .collect();
        l011.sort();
        assert_eq!(l011.len(), 3, "{l011:#?}");
        for missing in ["`config/gone.yaml`", "`maps`", "`scripts/tool.py`"] {
            assert!(
                l011.iter().any(|m| m.contains(missing)),
                "{missing}: {l011:#?}"
            );
        }
    }

    /// mavros_px4_events: `vendor/libevents` is a git submodule that this checkout did not
    /// initialise (an empty directory); its files are not missing from the repository.
    #[test]
    fn uninitialised_submodules_are_not_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p");
        std::fs::create_dir_all(p.join("vendor/libevents")).unwrap();
        std::fs::write(p.join("package.xml"), "<package format=\"3\"><name>p</name><export><build_type>ament_cmake</build_type></export></package>").unwrap();
        std::fs::write(p.join("CMakeLists.txt"), "project(p)\nfind_package(ament_cmake REQUIRED)\ninstall(FILES vendor/libevents/LICENSE.md DESTINATION share/${PROJECT_NAME})\nament_package()\n").unwrap();
        let d = crate::engine::analyze(
            dir.path(),
            &crate::config::Config::default(),
            &Default::default(),
        )
        .diagnostics;
        assert!(d.iter().all(|x| x.rule != "L011"), "{d:#?}");
    }

    #[test]
    fn python_subpackages_must_be_listed() {
        let pkg = "<package format=\"3\"><name>py</name><export><build_type>ament_python</build_type></export></package>";
        let by_hand = "from setuptools import setup\npackage_name = 'py'\nsetup(name=package_name, packages=[package_name])\n";
        let found = "from setuptools import setup, find_packages\nsetup(name='py', packages=find_packages(exclude=['test']))\n";
        let tree: &[(&str, &str)] = &[
            ("py/py/__init__.py", ""),
            ("py/py/core/__init__.py", ""),
            ("py/py/core/deep/__init__.py", ""),
            ("py/py/data/readme.txt", "not a package"),
            ("py/py/test/__init__.py", ""),
        ];
        let mut files = vec![("py/package.xml", pkg), ("py/setup.py", by_hand)];
        files.extend_from_slice(tree);
        let d = analyze_files(&files);
        let l012: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "L012")
            .map(|x| x.message.as_str())
            .collect();
        assert_eq!(l012.len(), 1, "{d:#?}");
        assert!(
            l012[0].contains("`py.core`")
                && l012[0].contains("`py.core.deep`")
                && !l012[0].contains("test"),
            "{l012:#?}"
        );
        let mut files = vec![("py/package.xml", pkg), ("py/setup.py", found)];
        files.extend_from_slice(tree);
        assert!(analyze_files(&files).iter().all(|x| x.rule != "L012"));
    }
}
