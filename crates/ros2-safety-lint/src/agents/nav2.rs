//! Nav2 agent: plugin wiring, frames, footprints and parameter typos in Nav2 YAML.

use super::{Agent, Context};
use crate::catalog::{
    nav2_params, normalize_plugin, NAV2_FRAME_PARAMS, NAV2_PLUGINS, NAV2_PLUGIN_LISTS, WORLD_FRAMES,
};
use crate::diag::Diagnostic;
use crate::facts::*;
use crate::util::{closest, edit_distance, max_typo_distance};
use std::collections::HashSet;

pub struct Nav2Agent;

pub(crate) const NAV2_NODES: &[&str] = &[
    "controller_server",
    "planner_server",
    "behavior_server",
    "smoother_server",
    "bt_navigator",
    "waypoint_follower",
    "velocity_smoother",
    "collision_monitor",
    "docking_server",
    "route_server",
    "amcl",
    "map_server",
    "local_costmap",
    "global_costmap",
    "lifecycle_manager",
];

fn leaf(k: &str) -> &str {
    k.rsplit('/').next().unwrap_or(k)
}

fn is_nav2_section(s: &ParamSection) -> bool {
    NAV2_NODES.contains(&leaf(&s.node_key))
        || s.params.iter().any(|e| {
            e.name.ends_with(".plugin")
                && matches!(&e.value, ParamValue::Str(v) if v.contains("nav2") || v.contains("dwb"))
        })
}

impl Agent for Nav2Agent {
    fn id(&self) -> &'static str {
        "nav2"
    }
    fn title(&self) -> &'static str {
        "Nav2 configuration"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        use crate::workspace::FileKind as K;
        // The launch graph plus robot descriptions (frames, footprint).
        &[
            K::Manifest,
            K::CMakeLists,
            K::SetupPy,
            K::SetupCfg,
            K::LaunchPy,
            K::LaunchXml,
            K::LaunchYaml,
            K::ParamYaml,
            K::Cpp,
            K::Python,
            K::Other,
            K::Urdf,
            K::BtXml,
        ]
    }
    fn relevant(&self, cx: &Context) -> bool {
        !cx.ix.bt_files.is_empty()
            || cx
                .ix
                .param_files
                .values()
                .any(|pf| pf.sections.iter().any(is_nav2_section))
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        let ws_plugins: HashSet<String> = cx
            .ix
            .packages
            .iter()
            .flat_map(|p| p.plugin_classes.iter().map(|c| normalize_plugin(c)))
            .chain(cx.ix.installed.all_plugin_classes().map(normalize_plugin))
            .collect();
        let known: HashSet<String> = NAV2_PLUGINS
            .iter()
            .map(|p| normalize_plugin(p))
            .chain(ws_plugins.iter().cloned())
            .collect();
        behavior_trees(cx, out);
        let frames = known_frames(cx);
        for pf in cx.ix.param_files.values() {
            for s in pf.sections.iter().filter(|s| is_nav2_section(s)) {
                let migrated = plugin_migration(cx, s, out);
                let before = out.len();
                plugin_sections(s, &known, out);
                // A value reported as renamed between distributions is not also a typo.
                let mut i = before;
                while i < out.len() {
                    if out[i].rule == "N002" && migrated.contains(&out[i].loc) {
                        out.remove(i);
                    } else {
                        i += 1;
                    }
                }
                if let Some(f) = &frames {
                    frame_params(s, f, out);
                }
                footprint(s, out);
                typos(s, out);
            }
        }
    }
}

/// N009: behavior-tree node types no one provides (near-typos of known types, or types of
/// another ROS distribution). N010: `<SubTree ID>` naming a tree no file defines. Known types:
/// BehaviorTree.CPP built-ins, the distribution's knowledge pack, registrations in workspace
/// code and `<TreeNodesModel>` declarations. Unknown types that look like nothing known are
/// not reported: they may come from a plugin library we cannot see.
fn behavior_trees(cx: &Context, out: &mut Vec<Diagnostic>) {
    if cx.ix.bt_files.is_empty() {
        return;
    }
    let distro = cx.ix.installed.distro.as_deref();
    // The analysed distribution's pack, including local packs; other distributions below use
    // only their embedded packs, so that every distribution is compared at equal coverage.
    let pack = distro.and(cx.ix.pack.as_ref());
    let mut known: HashSet<String> = crate::extract::bt::BUILTIN_NODES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let ws: Vec<String> = cx
        .ix
        .code
        .keys()
        .flat_map(|f| crate::pack::bt_registrations(&cx.ix.file(*f).text))
        .collect();
    if pack.is_none() && ws.is_empty() {
        return; // nothing to compare against
    }
    known.extend(ws);
    if let Some(p) = pack {
        known.extend(p.bt_nodes.iter().cloned());
    }
    let mut trees: HashSet<&str> = HashSet::new();
    let mut any_include = false;
    for b in cx.ix.bt_files.values() {
        known.extend(b.declared.iter().cloned());
        trees.extend(b.trees.iter().map(String::as_str));
        any_include |= b.has_include;
    }
    let others: Vec<(&str, &crate::pack::Pack)> = ["humble", "jazzy", "kilted"]
        .into_iter()
        .filter(|d| Some(*d) != distro)
        .filter_map(|d| crate::pack::cached(d).map(|p| (d, p)))
        .collect();
    for b in cx.ix.bt_files.values() {
        if cx.ix.external_files.contains(&b.file) {
            continue;
        }
        let mut seen = HashSet::new();
        for (id, loc) in &b.nodes {
            if known.contains(id) || !seen.insert(id.as_str()) {
                continue;
            }
            let elsewhere: Vec<&str> = others
                .iter()
                .filter(|(_, p)| p.bt_nodes.contains(id))
                .map(|(d, _)| *d)
                .collect();
            let near = closest(id, known.iter().map(String::as_str));
            if near.is_none() && elsewhere.is_empty() {
                continue;
            }
            let mut d = Diagnostic::new(
                "N009",
                *loc,
                format!("behavior-tree node type `{id}` is not provided by BehaviorTree.CPP, the Nav2 packages of this ROS distribution or this workspace; the tree fails to load"),
            );
            d = d.help(match (near, elsewhere.is_empty()) {
                (Some(c), true) => format!("Did you mean `{c}`?"),
                (Some(c), false) => format!(
                    "`{id}` exists in {}; did you mean `{c}`?",
                    elsewhere.join(", ")
                ),
                (None, false) => format!(
                    "`{id}` exists in {}: the tree was written for another release.",
                    elsewhere.join(", ")
                ),
                (None, true) => unreachable!(),
            });
            out.push(d);
        }
        // bt_navigator also registers trees from installed search directories, so an unknown ID
        // is only reported when it is a near-miss of a tree defined here (a typo).
        if !any_include {
            for (id, loc) in &b.subtree_refs {
                if trees.contains(id.as_str()) {
                    continue;
                }
                if let Some(c) = closest(id, trees.iter().copied()) {
                    out.push(
                        Diagnostic::new("N010", *loc, format!("`<SubTree ID=\"{id}\">` refers to a behavior tree that no tree file defines; the tree fails to load"))
                            .help(format!("Did you mean `{c}`?")),
                    );
                }
            }
        }
    }
}

/// Plugin names Nav2 renamed before Humble (Galactic/Foxy configurations), not in any pack.
const LEGACY_PLUGINS: &[(&str, &str)] = &[
    ("differential", "nav2_amcl::DifferentialMotionModel"),
    ("omnidirectional", "nav2_amcl::OmniMotionModel"),
];

/// N008: a `plugin:` (or AMCL `robot_model_type`) value that no package of this ROS
/// distribution exports, but another distribution's does (or a known legacy alias): the
/// plugin was renamed or removed, and the server fails to load it. Returns the reported
/// value locations. Message text has no distribution names (fingerprints must be stable).
fn plugin_migration(
    cx: &Context,
    s: &ParamSection,
    out: &mut Vec<Diagnostic>,
) -> HashSet<crate::source::Loc> {
    let mut done = HashSet::new();
    let Some(distro) = cx.ix.installed.distro.as_deref() else {
        return done;
    };
    let Some(here) = cx.ix.pack.as_ref() else {
        return done;
    };
    if here.plugins.is_empty() {
        return done;
    }
    let ws: HashSet<&str> = cx
        .ix
        .packages
        .iter()
        .flat_map(|p| p.plugin_classes.iter().map(String::as_str))
        .collect();
    for e in s
        .params
        .iter()
        .filter(|e| e.name.ends_with(".plugin") || e.name == "robot_model_type")
    {
        let ParamValue::Str(v) = &e.value else {
            continue;
        };
        let v = v.trim();
        if v.is_empty() || here.plugins.iter().any(|p| p == v) || ws.contains(v) {
            continue;
        }
        let legacy = LEGACY_PLUGINS
            .iter()
            .find(|(old, _)| *old == v)
            .map(|(_, new)| *new);
        let elsewhere: Vec<&str> = ["humble", "jazzy", "kilted"]
            .into_iter()
            .filter(|d| *d != distro && !(distro == "rolling" && *d == "kilted"))
            .filter(|d| crate::pack::cached(d).is_some_and(|p| p.plugins.iter().any(|x| x == v)))
            .collect();
        if legacy.is_none() && elsewhere.is_empty() {
            continue;
        }
        let suggestion = legacy
            .map(str::to_string)
            .or_else(|| closest(v, here.plugins.iter().map(String::as_str)).map(str::to_string));
        let mut d = Diagnostic::new(
            "N008",
            e.value_loc,
            format!("plugin `{v}` (`{}`) does not exist in this ROS distribution; it was renamed or removed, so the server fails to load it", e.name),
        );
        let origin = if legacy.is_some() {
            "an older Nav2 release".to_string()
        } else {
            elsewhere.join(", ")
        };
        d = d.help(match &suggestion {
            Some(c) => format!("Exists in {origin}; in {distro} use `{c}`."),
            None => format!("Exists in {origin}."),
        });
        done.insert(e.value_loc);
        out.push(d);
    }
    done
}

fn plugin_sections(s: &ParamSection, known: &HashSet<String>, out: &mut Vec<Diagnostic>) {
    for e in &s.params {
        let base = e.name.rsplit('.').next().unwrap_or(&e.name);
        if !NAV2_PLUGIN_LISTS.contains(&base) {
            continue;
        }
        let prefix = e.name.strip_suffix(base).unwrap_or("");
        let names: Vec<String> = match &e.value {
            ParamValue::StrArray(v) => v.clone(),
            ParamValue::Str(v) if base == "progress_checker_plugin" => vec![v.clone()],
            _ => continue,
        };
        for n in names {
            let plugin_key = format!("{prefix}{n}.plugin");
            if s.params.iter().find(|x| x.name == plugin_key).is_none() {
                let has_any = s
                    .params
                    .iter()
                    .any(|x| x.name.starts_with(&format!("{prefix}{n}.")));
                let msg = if has_any {
                    format!(
                        "`{n}` is listed in `{}` but its section has no `plugin:` type",
                        e.name
                    )
                } else {
                    format!("`{n}` is listed in `{}` but there is no `{n}:` section defining its `plugin:`", e.name)
                };
                out.push(Diagnostic::new(
                    "N001",
                    e.value_loc,
                    format!("{msg} (node `{}`)", s.node_key),
                ));
            }
        }
    }
    // Every `plugin:` value is checked, whether or not its section is listed.
    for p in s.params.iter().filter(|e| e.name.ends_with(".plugin")) {
        let ParamValue::Str(ty) = &p.value else {
            continue;
        };
        let norm = normalize_plugin(ty);
        if known.contains(&norm) {
            continue;
        }
        let best = known
            .iter()
            .map(|c| (edit_distance(&norm, c), c))
            .filter(|(d, _)| *d <= max_typo_distance(&norm).max(2))
            .min();
        if let Some((_, c)) = best {
            let replacement = if p.raw.is_some() {
                c.clone()
            } else {
                format!("\"{c}\"")
            };
            out.push(
                Diagnostic::new(
                    "N002",
                    p.value_loc,
                    format!("plugin type `{ty}` is not a known Nav2 plugin; did you mean `{c}`?"),
                )
                .safe_fix(
                    format!("Use `{c}`"),
                    p.value_loc.file,
                    p.value_loc.start,
                    p.value_loc.end,
                    replacement,
                ),
            );
        }
    }
}

/// Link names from complete robot models plus static transforms; None if unknown.
fn known_frames(cx: &Context) -> Option<HashSet<String>> {
    if cx.ix.robots.is_empty() || cx.ix.robots.iter().any(|r| r.partial) {
        return None;
    }
    let mut f: HashSet<String> = cx
        .ix
        .robots
        .iter()
        .flat_map(|r| r.links.iter().cloned())
        .collect();
    f.extend(WORLD_FRAMES.iter().map(|s| s.to_string()));
    for (_, _, n) in cx.graph.instances() {
        if n.exec.as_deref() == Some("static_transform_publisher") {
            for a in n.arguments.iter().flatten() {
                f.insert(a.clone());
            }
        }
    }
    Some(f)
}

fn frame_params(s: &ParamSection, frames: &HashSet<String>, out: &mut Vec<Diagnostic>) {
    for e in &s.params {
        let base = e.name.rsplit('.').next().unwrap_or(&e.name);
        if !NAV2_FRAME_PARAMS.contains(&base) {
            continue;
        }
        let ParamValue::Str(v) = &e.value else {
            continue;
        };
        let v = v.trim_start_matches('/');
        if v.is_empty() || v.contains('/') || v.contains("${") || frames.contains(v) {
            continue;
        }
        let mut d = Diagnostic::new(
            "N003",
            e.value_loc,
            format!(
                "`{}` = `{v}` is not a link in the robot description or a standard frame",
                e.name
            ),
        );
        if let Some(c) = closest(v, frames.iter().map(|s| s.as_str())) {
            let replacement = if e.raw.is_some() {
                c.to_string()
            } else {
                format!("\"{c}\"")
            };
            d = d.help(format!("Did you mean `{c}`?")).safe_fix(
                format!("Use `{c}`"),
                e.value_loc.file,
                e.value_loc.start,
                e.value_loc.end,
                replacement,
            );
        }
        // Repair site: the frame text (without quotes) may become any known frame.
        let span = if e.raw.is_some() {
            e.value_loc
        } else {
            crate::source::Loc::new(
                e.value_loc.file,
                e.value_loc.start + 1,
                e.value_loc.end.saturating_sub(1),
            )
        };
        d = d.repair(span, v, frames.iter().cloned().collect());
        out.push(d);
    }
}

fn parse_polygon(s: &str) -> Option<Vec<(f64, f64)>> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let inner = t.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let mut pts = Vec::new();
    for p in inner.split("],") {
        let p = p.trim_matches(|c| c == '[' || c == ']');
        let (x, y) = p.split_once(',')?;
        pts.push((x.parse().ok()?, y.parse().ok()?));
    }
    Some(pts)
}

fn footprint(s: &ParamSection, out: &mut Vec<Diagnostic>) {
    // Costmap parameters can live at the top level or under a nested costmap key.
    let prefixes: HashSet<String> = s
        .params
        .iter()
        .filter_map(|e| {
            e.name
                .strip_suffix("footprint")
                .or_else(|| e.name.strip_suffix("robot_radius"))
                .map(|p| p.to_string())
        })
        .collect();
    for pre in prefixes {
        let get = |n: &str| s.params.iter().find(|e| e.name == format!("{pre}{n}"));
        let mut radius = None;
        if let Some(r) = get("robot_radius") {
            if let Some(v) = r.value.as_f64() {
                if v <= 0.0 {
                    out.push(Diagnostic::new(
                        "N004",
                        r.value_loc,
                        format!("`robot_radius` is {v}; it must be positive"),
                    ));
                }
                radius = Some(v);
            }
        }
        if let Some(fp) = get("footprint") {
            if let ParamValue::Str(text) = &fp.value {
                match parse_polygon(text) {
                    None => out.push(Diagnostic::new(
                        "N004",
                        fp.value_loc,
                        format!("footprint `{text}` is not a list of [x, y] points"),
                    )),
                    Some(pts) if !pts.is_empty() && pts.len() < 3 => out.push(Diagnostic::new(
                        "N004",
                        fp.value_loc,
                        format!(
                            "footprint has {} point(s); a polygon needs at least 3",
                            pts.len()
                        ),
                    )),
                    Some(pts) if !pts.is_empty() => {
                        radius = Some(
                            pts.iter()
                                .map(|(x, y)| (x * x + y * y).sqrt())
                                .fold(0.0, f64::max),
                        );
                    }
                    _ => {}
                }
            }
        }
        // Inflation layers in the same costmap.
        let Some(r) = radius else { continue };
        for e in s
            .params
            .iter()
            .filter(|e| e.name.starts_with(&pre) && e.name.ends_with(".inflation_radius"))
        {
            let layer = e.name.strip_suffix(".inflation_radius").unwrap_or("");
            let is_inflation = s.params.iter().any(|x| {
                x.name == format!("{layer}.plugin")
                    && matches!(&x.value, ParamValue::Str(v) if v.contains("InflationLayer"))
            });
            if let (true, Some(ir)) = (is_inflation, e.value.as_f64()) {
                if ir < r {
                    out.push(Diagnostic::new(
                        "N005",
                        e.value_loc,
                        format!("inflation_radius {ir} is smaller than the robot's radius {r:.3}; paths can pass closer to obstacles than the robot is wide"),
                    ));
                }
            }
        }
    }
}

fn typos(s: &ParamSection, out: &mut Vec<Diagnostic>) {
    // Plugin sections: `<name>.<param>` checked against the plugin type's parameters.
    let plugins: Vec<(String, String)> = s
        .params
        .iter()
        .filter_map(|e| match (&e.value, e.name.strip_suffix(".plugin")) {
            (ParamValue::Str(t), Some(prefix)) => Some((prefix.to_string(), normalize_plugin(t))),
            _ => None,
        })
        .collect();
    let report = |e: &ParamEntry, name: &str, known: &[&str], out: &mut Vec<Diagnostic>| {
        if known.contains(&name) {
            return;
        }
        if let Some(c) = closest(name, known.iter().copied()) {
            let key_text_ok = true;
            let mut d = Diagnostic::new(
                "N006",
                e.key_loc,
                format!("`{name}` is not a parameter of this Nav2 component; did you mean `{c}`?"),
            );
            if key_text_ok {
                d = d.safe_fix(
                    format!("Rename to `{c}`"),
                    e.key_loc.file,
                    e.key_loc.start,
                    e.key_loc.end,
                    c,
                );
            }
            out.push(d);
        }
    };
    for e in &s.params {
        // plugin params
        if let Some((prefix, ty)) = plugins
            .iter()
            .find(|(p, _)| e.name.starts_with(&format!("{p}.")))
        {
            let rest = &e.name[prefix.len() + 1..];
            if !rest.contains('.') {
                report(e, rest, nav2_params(ty), out);
            }
            continue;
        }
        if e.name.contains('.') {
            continue;
        }
        let kind = match leaf(&s.node_key) {
            "local_costmap" | "global_costmap" => "costmap",
            "controller_server" => "controller_server",
            "amcl" => "amcl",
            _ => continue,
        };
        report(e, &e.name, nav2_params(kind), out);
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn nav2_checks() {
        let yaml = r#"
controller_server:
  ros__parameters:
    controller_frequncy: 20.0
    controller_plugins: ["FollowPath", "Missing"]
    FollowPath:
      plugin: "nav2_regulated_pure_pursuit_controller::RegulatedPurePursuitControler"
      desired_linear_vel: 0.5
local_costmap:
  local_costmap:
    ros__parameters:
      global_frame: odom
      robot_base_frame: base_lnk
      robot_radius: 0.3
      plugins: ["inflation_layer"]
      inflation_layer:
        plugin: "nav2_costmap_2d::InflationLayer"
        inflation_radius: 0.1
        cost_scalling_factor: 3.0
"#;
        let urdf = r#"<robot name="r"><link name="base_link"/></robot>"#;
        let d = analyze_files(&[
            (
                "p/package.xml",
                "<package format=\"3\"><name>p</name></package>",
            ),
            ("p/config/nav2.yaml", yaml),
            ("p/urdf/r.urdf", urdf),
        ]);
        let ids: Vec<&str> = d.iter().map(|x| x.rule).collect();
        assert!(ids.contains(&"N001"), "{d:#?}");
        assert!(ids.contains(&"N002"), "{d:#?}");
        assert!(ids.contains(&"N003"), "{d:#?}");
        assert!(ids.contains(&"N005"), "{d:#?}");
        assert_eq!(ids.iter().filter(|r| **r == "N006").count(), 2, "{d:#?}");
    }

    /// N008 (legacy AMCL alias), N009 (BT node typo) and N010 (subtree typo), with a known
    /// distribution; valid names stay silent.
    #[test]
    fn plugin_migration_and_behavior_trees() {
        let dir = tempfile::tempdir().unwrap();
        let w = |r: &str, t: &str| {
            let p = dir.path().join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "bt/package.xml",
            "<package format=\"3\"><name>bt</name></package>",
        );
        w("bt/config/nav2.yaml", "amcl:\n  ros__parameters:\n    robot_model_type: \"differential\"\nplanner_server:\n  ros__parameters:\n    planner_plugins: [\"GridBased\"]\n    GridBased:\n      plugin: \"nav2_navfn_planner::NavfnPlanner\"\n");
        w("bt/behavior_trees/nav.xml", "<root BTCPP_format=\"4\" main_tree_to_execute=\"Main\">\n  <BehaviorTree ID=\"Main\">\n    <Sequence>\n      <ComputePathToPos goal=\"{goal}\"/>\n      <FollowPath path=\"{path}\"/>\n      <SubTree ID=\"Recovry\"/>\n      <MyCustomNode/>\n    </Sequence>\n  </BehaviorTree>\n  <BehaviorTree ID=\"Recovery\"><Wait wait_duration=\"1\"/></BehaviorTree>\n</root>\n");
        let opts = crate::engine::Options {
            distro: Some("jazzy".into()),
            ..Default::default()
        };
        let d = crate::engine::analyze(dir.path(), &crate::config::Config::default(), &opts)
            .diagnostics;
        let by = |r: &str| d.iter().filter(|x| x.rule == r).collect::<Vec<_>>();
        let n008 = by("N008");
        assert!(
            n008.len() == 1
                && n008[0].message.contains("`differential`")
                && n008[0]
                    .help
                    .as_deref()
                    .is_some_and(|h| h.contains("nav2_amcl::DifferentialMotionModel")),
            "{d:#?}"
        );
        assert!(
            !n008[0].message.to_lowercase().contains("jazzy"),
            "no distro names in messages"
        );
        let n009 = by("N009");
        assert!(
            n009.len() == 1 && n009[0].message.contains("ComputePathToPos"),
            "{d:#?}"
        );
        let n010 = by("N010");
        assert!(
            n010.len() == 1
                && n010[0]
                    .help
                    .as_deref()
                    .is_some_and(|h| h.contains("Recovery")),
            "{d:#?}"
        );
    }
}
