//! package.xml, setup.py, setup.cfg, plugin description XML and
//! generate_parameter_library spec files.

use super::yaml::{self, YNode};
use crate::facts::{
    BuildType, Constraint, DepKind, Dependency, Executable, NamePattern, ParamDecl, ParamType,
    PythonPackageInfo,
};
use crate::source::{FileId, Loc};
use rustpython_parser::ast::{self, Constant, Expr, Ranged, Stmt};

#[derive(Debug, Default)]
pub struct Manifest {
    pub name: String,
    pub name_loc: (usize, usize),
    pub deps: Vec<Dependency>,
    pub build_type: Option<BuildType>,
    pub plugin_xmls: Vec<String>,
    /// Some dependency is conditional on `$ROS_VERSION == 1`: the package also builds for ROS 1.
    pub ros1_support: bool,
}

/// Whether a REP 149 `condition` holds on ROS 2. Only comparisons of `$ROS_VERSION` are
/// evaluated; anything else (distribution conditions, and/or) counts as true.
fn condition_holds_on_ros2(cond: &str) -> bool {
    let c: String = cond
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect();
    if c.contains("and") || c.contains("or") {
        return true;
    }
    match (
        c.strip_prefix("$ROS_VERSION==")
            .or_else(|| c.strip_suffix("==$ROS_VERSION")),
        c.strip_prefix("$ROS_VERSION!=")
            .or_else(|| c.strip_suffix("!=$ROS_VERSION")),
    ) {
        (Some(v), _) => v == "2",
        (_, Some(v)) => v != "2",
        _ => true,
    }
}

pub fn parse_manifest(file: FileId, text: &str) -> Result<Manifest, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let mut m = Manifest::default();
    let root = doc.root_element();
    if !root.has_tag_name("package") {
        return Err("root element is not <package>".into());
    }
    for n in root.children().filter(|n| n.is_element()) {
        let tag = n.tag_name().name();
        let txt = n.text().unwrap_or("").trim().to_string();
        let r = n.range();
        let kind = match tag {
            "name" => {
                m.name = txt.clone();
                m.name_loc = (r.start, r.end);
                None
            }
            "depend" => Some(DepKind::Depend),
            "build_depend" => Some(DepKind::Build),
            "build_export_depend" => Some(DepKind::BuildExport),
            "exec_depend" | "run_depend" => Some(DepKind::Exec),
            "test_depend" => Some(DepKind::Test),
            "buildtool_depend" => Some(DepKind::BuildTool),
            "export" => {
                for e in n.children().filter(|c| c.is_element()) {
                    if e.has_tag_name("build_type") {
                        m.build_type = Some(match e.text().unwrap_or("").trim() {
                            "ament_cmake" => BuildType::AmentCmake,
                            "ament_python" => BuildType::AmentPython,
                            "cmake" => BuildType::Cmake,
                            _ => BuildType::Unknown,
                        });
                    }
                    if let Some(p) = e.attribute("plugin") {
                        m.plugin_xmls
                            .push(p.replace("${prefix}/", "").replace("${prefix}", ""));
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(kind) = kind {
            // REP 149: a dependency for ROS 1 only is not a dependency on ROS 2.
            if let Some(cond) = n.attribute("condition") {
                if !condition_holds_on_ros2(cond) {
                    m.ros1_support = true;
                    continue;
                }
            }
            if !txt.is_empty() {
                m.deps.push(Dependency {
                    name: txt,
                    kind,
                    loc: Loc::new(file, r.start, r.end),
                });
            }
        }
    }
    if m.name.is_empty() {
        return Err("package.xml has no <name>".into());
    }
    Ok(m)
}

/// Plugin lookup names and types declared in a pluginlib description file.
pub fn parse_plugin_xml(text: &str) -> Vec<String> {
    let Ok(doc) = roxmltree::Document::parse(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for n in doc.descendants().filter(|n| n.has_tag_name("class")) {
        for a in ["name", "type"] {
            if let Some(v) = n.attribute(a) {
                out.push(v.to_string());
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------- setup.py / setup.cfg

#[derive(Debug, Default)]
pub struct SetupPyFacts {
    pub executables: Vec<Executable>,
    pub share_dirs: Vec<String>,
    pub data_files_complete: bool,
    pub console_scripts_loc: Option<Loc>,
    /// The literal `packages=[...]` list and its location; None when computed or absent.
    pub packages: Option<(Vec<String>, Loc)>,
}

/// Extracts console_scripts and data_files destinations from setup.py.
pub fn parse_setup_py(
    file: FileId,
    text: &str,
    pkg_name: &str,
    pkg_dir: &str,
) -> Result<SetupPyFacts, String> {
    let suite = rustpython_parser::parse(text, rustpython_parser::Mode::Module, "setup.py")
        .map_err(|e| e.to_string())?;
    let ast::Mod::Module(m) = suite else {
        return Err("not a module".into());
    };
    let mut f = SetupPyFacts {
        data_files_complete: true,
        ..Default::default()
    };
    let mut consts: Vec<(String, String)> = vec![("package_name".into(), pkg_name.to_string())];
    for stmt in &m.body {
        if let Stmt::Assign(a) = stmt {
            if let (Some(Expr::Name(n)), Some(v)) = (a.targets.first(), str_expr(&a.value, &consts))
            {
                consts.push((n.id.to_string(), v));
            }
        }
    }
    let mut setup_calls = Vec::new();
    for stmt in &m.body {
        collect_setup_calls(stmt, &mut setup_calls);
    }
    let mut package_dir = false;
    for call in setup_calls {
        for kw in &call.keywords {
            let Some(arg) = kw.arg.as_ref().map(|a| a.as_str()) else {
                continue;
            };
            match arg {
                "data_files" => match &kw.value {
                    Expr::List(l) => {
                        for e in &l.elts {
                            let dest = match e {
                                Expr::Tuple(t) => t.elts.first().and_then(|d| str_expr(d, &consts)),
                                _ => None,
                            };
                            match dest {
                                Some(d) => {
                                    let d = d.replace('\\', "/");
                                    let d = d.trim_matches('/');
                                    if let Some(sub) = d.strip_prefix(&format!("share/{pkg_name}/"))
                                    {
                                        f.share_dirs
                                            .push(sub.split('/').next().unwrap_or(sub).to_string());
                                    } else if d == format!("share/{pkg_name}") && !only_manifest(e)
                                    {
                                        // Content installed to the share root: subdirectory layout unknown.
                                        f.share_dirs.push(".".into());
                                    }
                                }
                                None => f.data_files_complete = false,
                            }
                        }
                    }
                    _ => f.data_files_complete = false,
                },
                "entry_points" => {
                    if let Expr::Dict(d) = &kw.value {
                        for (k, v) in d.keys.iter().zip(&d.values) {
                            let is_console =
                                k.as_ref().and_then(|k| str_expr(k, &consts)).as_deref()
                                    == Some("console_scripts");
                            if !is_console {
                                continue;
                            }
                            f.console_scripts_loc = Some(Loc::new(
                                file,
                                v.range().start().to_usize(),
                                v.range().end().to_usize(),
                            ));
                            if let Expr::List(l) = v {
                                for e in &l.elts {
                                    let Some(s) = str_expr(e, &consts) else {
                                        continue;
                                    };
                                    let Some((name, target)) = s.split_once('=') else {
                                        continue;
                                    };
                                    let module =
                                        target.trim().split(':').next().unwrap_or("").trim();
                                    let path = module.replace('.', "/");
                                    let src = if pkg_dir.is_empty() {
                                        format!("{path}.py")
                                    } else {
                                        format!("{pkg_dir}/{path}.py")
                                    };
                                    f.executables.push(Executable {
                                        name: name.trim().to_string(),
                                        sources: vec![src],
                                        loc: Loc::new(
                                            file,
                                            e.range().start().to_usize(),
                                            e.range().end().to_usize(),
                                        ),
                                        installed: true,
                                    });
                                }
                            }
                        }
                    }
                }
                "packages" => {
                    let loc = Loc::new(
                        file,
                        kw.value.range().start().to_usize(),
                        kw.value.range().end().to_usize(),
                    );
                    f.packages = match &kw.value {
                        Expr::List(l) => l
                            .elts
                            .iter()
                            .map(|e| str_expr(e, &consts))
                            .collect::<Option<Vec<String>>>()
                            .map(|v| (v, loc)),
                        _ => None, // find_packages(...) or anything computed
                    };
                }
                // Packages live elsewhere than `<name>/`: the layout is not ours to check.
                "package_dir" => package_dir = true,
                _ => {}
            }
        }
    }
    if package_dir {
        f.packages = None;
    }
    Ok(f)
}

/// True if a data_files entry installs nothing but package.xml / resource markers.
fn only_manifest(e: &Expr) -> bool {
    let Expr::Tuple(t) = e else { return false };
    let Some(Expr::List(l)) = t.elts.get(1) else {
        return false;
    };
    l.elts.iter().all(|x| matches!(x, Expr::Constant(c) if matches!(&c.value, Constant::Str(s) if s.ends_with("package.xml"))))
}

fn collect_setup_calls<'a>(stmt: &'a Stmt, out: &mut Vec<&'a ast::ExprCall>) {
    match stmt {
        Stmt::Expr(e) => {
            if let Expr::Call(c) = &*e.value {
                let name = match &*c.func {
                    Expr::Name(n) => n.id.as_str(),
                    Expr::Attribute(a) => a.attr.as_str(),
                    _ => "",
                };
                if name == "setup" {
                    out.push(c);
                }
            }
        }
        Stmt::If(i) => {
            for s in i.body.iter().chain(&i.orelse) {
                collect_setup_calls(s, out);
            }
        }
        _ => {}
    }
}

/// Statically evaluates simple string expressions used in setup.py.
fn str_expr(e: &Expr, consts: &[(String, String)]) -> Option<String> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => Some(s.clone()),
            _ => None,
        },
        Expr::Name(n) => consts
            .iter()
            .rev()
            .find(|(k, _)| k == n.id.as_str())
            .map(|(_, v)| v.clone()),
        Expr::BinOp(b) if matches!(b.op, ast::Operator::Add) => Some(format!(
            "{}{}",
            str_expr(&b.left, consts)?,
            str_expr(&b.right, consts)?
        )),
        Expr::JoinedStr(j) => {
            let mut s = String::new();
            for v in &j.values {
                match v {
                    Expr::FormattedValue(fv) => s.push_str(&str_expr(&fv.value, consts)?),
                    other => s.push_str(&str_expr(other, consts)?),
                }
            }
            Some(s)
        }
        Expr::Call(c) => {
            let fname = match &*c.func {
                Expr::Attribute(a) => a.attr.as_str(),
                Expr::Name(n) => n.id.as_str(),
                _ => return None,
            };
            if fname == "join" {
                let parts: Option<Vec<String>> =
                    c.args.iter().map(|a| str_expr(a, consts)).collect();
                Some(parts?.join("/"))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// setup.cfg must route console scripts to `lib/<pkg>`.
pub fn setup_cfg_ok(text: &str) -> bool {
    let mut section = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            section = l[1..l.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        if section == "install" {
            if let Some((k, v)) = l.split_once('=') {
                let k = k.trim().replace('-', "_");
                if k == "install_scripts" && v.contains("lib/") {
                    return true;
                }
            }
        }
    }
    false
}

pub fn python_info(
    setup_py: Option<FileId>,
    setup_cfg: Option<(FileId, &str)>,
    sp: Option<&SetupPyFacts>,
) -> PythonPackageInfo {
    PythonPackageInfo {
        setup_py,
        setup_cfg: setup_cfg.map(|(id, _)| id),
        setup_cfg_install_scripts_ok: setup_cfg.is_some_and(|(_, t)| setup_cfg_ok(t)),
        has_console_scripts: sp.is_some_and(|s| !s.executables.is_empty()),
        console_scripts_loc: sp.and_then(|s| s.console_scripts_loc),
    }
}

// ---------------------------------------------------------------------------- generate_parameter_library

/// Parameters declared through a generate_parameter_library YAML spec.
/// Returns (declarations, open) where `open` means some names are dynamic.
pub fn parse_param_lib_spec(file: FileId, text: &str) -> (Vec<ParamDecl>, bool) {
    let Ok(docs) = yaml::parse(text) else {
        return (Vec::new(), true);
    };
    let mut out = Vec::new();
    let mut open = false;
    for doc in &docs {
        for (_ns, body) in doc.entries() {
            walk_spec(file, body, "", false, &mut out, &mut open);
        }
    }
    (out, open)
}

fn walk_spec(
    file: FileId,
    node: &YNode,
    prefix: &str,
    dynamic: bool,
    out: &mut Vec<ParamDecl>,
    open: &mut bool,
) {
    for (k, v) in node.entries() {
        let Some(key) = k.as_str() else { continue };
        if let Some(rest) = key.strip_prefix("__map_") {
            // `__map_joints:` repeats its children for every entry of the `joints` parameter.
            // Names under it are `<prefix>.<entry>.<child>`: match them by their literal tail.
            let _ = rest;
            *open = true;
            walk_spec(file, v, "", true, out, open);
            continue;
        }
        let name = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        if let Some(t) = v.get("type").and_then(|t| t.as_str()) {
            let ty = match t {
                "bool" => ParamType::Bool,
                "int" => ParamType::Int,
                "double" => ParamType::Double,
                "string" => ParamType::Str,
                "bool_array" => ParamType::BoolArray,
                "int_array" => ParamType::IntArray,
                "double_array" => ParamType::DoubleArray,
                "string_array" => ParamType::StrArray,
                t if t.starts_with("string_fixed") => ParamType::Str,
                t if t.starts_with("double_array_fixed") => ParamType::DoubleArray,
                t if t.starts_with("int_array_fixed") => ParamType::IntArray,
                t if t.starts_with("string_array_fixed") => ParamType::StrArray,
                _ => ParamType::Unknown,
            };
            let pattern = if dynamic {
                NamePattern::Suffix(format!(".{name}"))
            } else {
                NamePattern::Exact(name.clone())
            };
            out.push(ParamDecl {
                name: pattern,
                ty,
                has_default: v.get("default_value").is_some(),
                default_text: None,
                loc: Loc::new(file, k.start(), k.end()),
                constraints: v
                    .get("validation")
                    .map(spec_constraints)
                    .unwrap_or_default(),
            });
        } else if v.is_map() {
            walk_spec(file, v, &name, dynamic, out, open);
        }
    }
}

/// generate_parameter_library validators the node enforces at startup. Unknown or custom
/// validators (e.g. `my_pkg::check<>`) and element-wise ones are ignored.
fn spec_constraints(v: &YNode) -> Vec<Constraint> {
    let num = |n: &YNode| n.as_str().and_then(|s| s.trim().parse::<f64>().ok());
    let mut out = Vec::new();
    for (k, args) in v.entries() {
        let Some(name) = k.as_str() else { continue };
        let a = args.items();
        let c = match name {
            "bounds<>" => match (a.first().and_then(num), a.get(1).and_then(num)) {
                (Some(min), Some(max)) => Some(Constraint::Range { min, max }),
                _ => None,
            },
            "gt<>" | "gt_eq<>" => a.first().and_then(num).map(|b| Constraint::Lower {
                bound: b,
                inclusive: name == "gt_eq<>",
            }),
            "lt<>" | "lt_eq<>" => a.first().and_then(num).map(|b| Constraint::Upper {
                bound: b,
                inclusive: name == "lt_eq<>",
            }),
            "one_of<>" => a
                .first()
                .map(|l| {
                    Constraint::OneOf(
                        l.items()
                            .iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect(),
                    )
                })
                .filter(|c| !matches!(c, Constraint::OneOf(v) if v.is_empty())),
            "not_empty<>" => Some(Constraint::NotEmpty),
            "size_lt<>" => a.first().and_then(num).map(|n| Constraint::Size {
                min: 0,
                max: (n as usize).saturating_sub(1),
            }),
            "size_gt<>" => a.first().and_then(num).map(|n| Constraint::Size {
                min: n as usize + 1,
                max: usize::MAX,
            }),
            "fixed_size<>" => a.first().and_then(num).map(|n| Constraint::Size {
                min: n as usize,
                max: n as usize,
            }),
            _ => None,
        };
        out.extend(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_validators() {
        let spec = "ctrl:
  wheel_separation:
    type: double
    default_value: 0.3
    validation:
      gt<>: [0.0]
  mode:
    type: string
    default_value: a
    validation:
      one_of<>: [[\"a\", \"b\"]]
      custom::check<>: []
  gain:
    type: double
    default_value: 1.0
    validation:
      bounds<>: [0.0, 10.0]
";
        let (d, _) = parse_param_lib_spec(0, spec);
        let get = |n: &str| {
            d.iter()
                .find(|x| x.name.literal() == n)
                .map(|x| x.constraints.clone())
                .unwrap()
        };
        assert_eq!(
            get("wheel_separation"),
            vec![Constraint::Lower {
                bound: 0.0,
                inclusive: false
            }]
        );
        assert_eq!(
            get("mode"),
            vec![Constraint::OneOf(vec!["a".into(), "b".into()])]
        );
        assert_eq!(
            get("gain"),
            vec![Constraint::Range {
                min: 0.0,
                max: 10.0
            }]
        );
    }

    #[test]
    fn manifest() {
        let m = parse_manifest(0, r#"<package format="3"><name>p</name><depend>rclcpp</depend><exec_depend>nav2_bringup</exec_depend><export><build_type>ament_python</build_type><nav2_core plugin="${prefix}/plugins.xml"/></export></package>"#).unwrap();
        assert_eq!(m.name, "p");
        assert_eq!(m.deps.len(), 2);
        assert_eq!(m.build_type, Some(BuildType::AmentPython));
        assert_eq!(m.plugin_xmls, vec!["plugins.xml"]);
    }

    #[test]
    fn conditional_dependencies() {
        let m = parse_manifest(
            0,
            r#"<package format="3"><name>p</name>
<buildtool_depend condition="$ROS_VERSION == 1">catkin</buildtool_depend>
<buildtool_depend condition="$ROS_VERSION == 2">ament_cmake</buildtool_depend>
<depend condition="$ROS_VERSION != 1">rclcpp</depend>
<depend condition="$ROS_DISTRO == humble">foo</depend></package>"#,
        )
        .unwrap();
        let names: Vec<&str> = m.deps.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["ament_cmake", "rclcpp", "foo"]);
        assert!(m.ros1_support);
        assert!(
            !parse_manifest(
                0,
                "<package format=\"3\"><name>q</name><depend>rclcpp</depend></package>"
            )
            .unwrap()
            .ros1_support
        );
    }

    #[test]
    fn setup_py_entry_points_and_data() {
        let src = r#"
import os
from glob import glob
from setuptools import setup
package_name = 'my_pkg'
setup(
    name=package_name,
    data_files=[
        ('share/ament_index/resource_index/packages', ['resource/' + package_name]),
        ('share/' + package_name, ['package.xml']),
        (os.path.join('share', package_name, 'launch'), glob('launch/*.py')),
    ],
    entry_points={'console_scripts': ['talker = my_pkg.talker:main']},
)
"#;
        let f = parse_setup_py(0, src, "my_pkg", "src/my_pkg").unwrap();
        assert_eq!(f.executables[0].name, "talker");
        assert_eq!(f.executables[0].sources[0], "src/my_pkg/my_pkg/talker.py");
        assert_eq!(f.share_dirs, vec!["launch"]);
        assert!(f.data_files_complete);
    }

    #[test]
    fn setup_cfg_check() {
        assert!(setup_cfg_ok(
            "[develop]\nscript_dir=$base/lib/p\n[install]\ninstall_scripts=$base/lib/p\n"
        ));
        assert!(setup_cfg_ok("[install]\ninstall-scripts = $base/lib/p\n"));
        assert!(!setup_cfg_ok("[develop]\nscript_dir=$base/lib/p\n"));
    }

    #[test]
    fn param_lib_spec() {
        let src = "jtc:\n  joints:\n    type: string_array\n    default_value: []\n  gains:\n    __map_joints:\n      p:\n        type: double\n";
        let (d, open) = parse_param_lib_spec(0, src);
        assert!(open);
        assert!(d
            .iter()
            .any(|p| p.name == NamePattern::Exact("joints".into()) && p.ty == ParamType::StrArray));
        assert!(d
            .iter()
            .any(|p| p.name == NamePattern::Suffix(".gains.p".into())
                || p.name == NamePattern::Suffix(".p".into())));
    }
}
