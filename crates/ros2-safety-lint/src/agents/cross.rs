//! Cross-parameter constraints (P008): values that are each valid on their own but contradict
//! each other, so the node ignores one of them or misbehaves.
//!
//! Each constraint is one the node's own code enforces or exhibits; the comment above it
//! names the upstream source. Values not set anywhere take the node's declared default; a
//! value that cannot be determined (an unread parameter source, a substitution) turns the
//! check off for that node.

use super::Context;
use crate::diag::Diagnostic;
use crate::facts::ParamValue;
use crate::resolve::NodeInstance;
use crate::rules::Severity;
use crate::source::Loc;

pub fn run(cx: &Context, out: &mut Vec<Diagnostic>) {
    for (_, _, n) in cx.graph.instances() {
        robot_localization_frames(n, out);
    }
    costmap_footprint_and_radius(cx, out);
}

/// A string parameter's effective value: as set (with where), else the default.
/// None when it is set to something that is not a known string.
fn string_param(n: &NodeInstance, name: &str, default: Option<&str>) -> Option<(String, Option<Loc>)> {
    match n.param(name) {
        Some(b) => match &b.value {
            Some(ParamValue::Str(s)) => Some((s.trim_start_matches('/').to_string(), Some(b.value_loc))),
            _ => None,
        },
        None => default.map(|d| (d.to_string(), None)),
    }
}

/// robot_localization (src/ros_filter.cpp, `RosFilter::loadParams`): map_frame, odom_frame
/// and base_link_frame must all differ, and base_link_frame_output must differ from the
/// first two. Otherwise the node logs "Invalid frame configuration!" and keeps running with a
/// frame setup in which it publishes a transform from a frame to itself or to the wrong
/// parent. Defaults: map, odom, base_link; base_link_frame_output = base_link_frame.
fn robot_localization_frames(n: &NodeInstance, out: &mut Vec<Diagnostic>) {
    if n.pkg.as_deref() != Some("robot_localization") || !matches!(n.exec.as_deref(), Some("ekf_node" | "ukf_node")) {
        return;
    }
    if !n.param_sources_complete {
        return;
    }
    let Some(map) = string_param(n, "map_frame", Some("map")) else { return };
    let Some(odom) = string_param(n, "odom_frame", Some("odom")) else { return };
    let Some(base) = string_param(n, "base_link_frame", Some("base_link")) else { return };
    let Some(output) = string_param(n, "base_link_frame_output", Some(&base.0.clone())) else { return };
    let frames = [("map_frame", &map), ("odom_frame", &odom), ("base_link_frame", &base), ("base_link_frame_output", &output)];
    let pairs = [(0, 1), (1, 2), (0, 2), (1, 3), (0, 3)];
    for (a, b) in pairs {
        let ((na, va), (nb, vb)) = (frames[a], frames[b]);
        if va.0 != vb.0 {
            continue;
        }
        // Point at whichever of the two was written in the configuration.
        let loc = vb.1.or(va.1).unwrap_or(n.decl.loc);
        out.push(
            Diagnostic::new(
                "P008",
                loc,
                format!(
                    "`{na}` and `{nb}` of `{}` are both `{}`; robot_localization requires map_frame, odom_frame and base_link_frame to differ, logs \"Invalid frame configuration\" and keeps running with a broken transform tree",
                    n.display_name(),
                    va.0
                ),
            )
            .help("Give each of map_frame, odom_frame and base_link_frame its own frame (defaults: map, odom, base_link)."),
        );
        return;
    }
}

/// Nav2 costmaps (nav2_costmap_2d/src/costmap_2d_ros.cpp, `Costmap2DROS::getParameters`):
/// a valid `footprint` replaces the circular robot: `robot_radius` is then never used.
fn costmap_footprint_and_radius(cx: &Context, out: &mut Vec<Diagnostic>) {
    for pf in cx.ix.param_files.values() {
        if cx.ix.external_files.contains(&pf.file) {
            continue;
        }
        for s in &pf.sections {
            let leaf = s.node_key.rsplit('/').next().unwrap_or("");
            if !leaf.ends_with("costmap") {
                continue;
            }
            let get = |name: &str| s.params.iter().find(|e| e.name == name);
            let (Some(fp), Some(radius)) = (get("footprint"), get("robot_radius")) else { continue };
            let ParamValue::Str(text) = &fp.value else { continue };
            // makeFootprintFromString needs at least three [x, y] points.
            let points = text.matches('[').count().saturating_sub(1);
            if points < 3 {
                continue;
            }
            out.push(
                Diagnostic::new(
                    "P008",
                    radius.key_loc,
                    format!("`robot_radius` of costmap `{}` is ignored: the costmap also sets a `footprint`, which Nav2 uses instead", s.node_key),
                )
                .severity(Severity::Info)
                .related(fp.key_loc, "footprint set here")
                .help("Remove `robot_radius` (or the footprint, to model the robot as a circle)."),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    const PKG: (&str, &str) = ("b/package.xml", "<package format=\"3\"><name>b</name><exec_depend>robot_localization</exec_depend></package>");

    fn ekf(yaml: &str) -> Vec<crate::diag::Diagnostic> {
        let launch = "<launch><node pkg=\"robot_localization\" exec=\"ekf_node\" name=\"ekf\"><param from=\"$(find-pkg-share b)/config/ekf.yaml\"/></node></launch>";
        analyze_files(&[PKG, ("b/config/ekf.yaml", yaml), ("b/launch/a.launch.xml", launch)])
    }

    #[test]
    fn robot_localization_frames_must_differ() {
        let d = ekf("ekf:\n  ros__parameters:\n    world_frame: odom\n    odom_frame: base_link\n");
        let p008: Vec<&str> = d.iter().filter(|x| x.rule == "P008").map(|x| x.message.as_str()).collect();
        assert_eq!(p008.len(), 1, "{d:#?}");
        assert!(p008[0].contains("`odom_frame` and `base_link_frame`") && p008[0].contains("`base_link`"), "{p008:#?}");
        // Defaults and distinct values are fine.
        assert!(ekf("ekf:\n  ros__parameters:\n    two_d_mode: true\n").iter().all(|x| x.rule != "P008"));
        assert!(ekf("ekf:\n  ros__parameters:\n    base_link_frame: base_footprint\n").iter().all(|x| x.rule != "P008"));
    }

    #[test]
    fn footprint_makes_robot_radius_dead() {
        let both = "local_costmap:\n  local_costmap:\n    ros__parameters:\n      footprint: \"[[0.2, 0.2], [0.2, -0.2], [-0.2, -0.2], [-0.2, 0.2]]\"\n      robot_radius: 0.3\n";
        let d = analyze_files(&[("n/package.xml", "<package format=\"3\"><name>n</name></package>"), ("n/config/nav.yaml", both)]);
        let p: Vec<_> = d.iter().filter(|x| x.rule == "P008").collect();
        assert_eq!(p.len(), 1, "{d:#?}");
        assert_eq!(p[0].severity, crate::rules::Severity::Info);
        let empty = "local_costmap:\n  local_costmap:\n    ros__parameters:\n      footprint: \"[]\"\n      robot_radius: 0.3\n";
        let d = analyze_files(&[("n/package.xml", "<package format=\"3\"><name>n</name></package>"), ("n/config/nav.yaml", empty)]);
        assert!(d.iter().all(|x| x.rule != "P008"), "{d:#?}");
    }
}
