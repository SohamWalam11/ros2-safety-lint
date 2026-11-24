//! ROS 1 agent: ROS 1 leftovers in ROS 2 packages.
//!
//! - O001 a ROS 1 XML launch file (`<node type=…>`, `<rosparam>`, `$(find pkg)`) in a ROS 2
//!   package: `ros2 launch` rejects it;
//! - O002 a Python module importing a ROS 1 library (`rospy`, `roslib`, `rosbag`, `tf`,
//!   `actionlib`, `dynamic_reconfigure`) or C++ including `ros/ros.h`, in a ROS 2 package
//!   and without any ROS 2 client library: it fails at import or does not compile;
//! - O003 a catkin package in a workspace of ROS 2 packages: colcon builds it with catkin,
//!   which fails where only ROS 2 is installed.
//!
//! A package counts as ROS 1 when it depends on `catkin` (dependencies conditional on
//! `$ROS_VERSION == 1` do not count on ROS 2). Packages that declare such conditions build
//! for both versions, so their ROS 1 files are intentional and not reported. Directories
//! with COLCON_IGNORE, AMENT_IGNORE or CATKIN_IGNORE are never read; vendored directories
//! (third_party, vendor, external) and hidden ones (.github) are skipped, and so are
//! packages nested in another package, which colcon does not see. Python that guards its
//! imports (`except ImportError`) to support both versions is not reported.

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::facts::Package;
use crate::source::{FileId, Loc};
use crate::workspace::FileKind;

pub struct Ros1Agent;

/// Python modules that exist only in ROS 1.
const ROS1_PY: &[&str] = &["rospy", "roslib", "rosbag", "tf", "actionlib", "dynamic_reconfigure", "rosgraph", "rostest", "roslaunch"];

const MIGRATE_LAUNCH: &str = "https://docs.ros.org/en/jazzy/How-To-Guides/Migrating-from-ROS1/Migrating-Launch-Files.html";

/// The first ROS 1 module a Python file imports: (module, byte offset of the import line).
pub fn ros1_python_import(text: &str) -> Option<(&'static str, usize)> {
    if text.contains("rclpy") || text.contains("except ImportError") || text.contains("except ModuleNotFoundError") {
        return None;
    }
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        let module = t
            .strip_prefix("import ")
            .or_else(|| t.strip_prefix("from ").and_then(|r| r.split_once(" import").map(|(m, _)| m)))
            .map(|m| m.split([',', ' ', '.']).next().unwrap_or(""));
        if let Some(m) = module {
            if let Some(hit) = ROS1_PY.iter().find(|r| **r == m) {
                return Some((hit, offset + (line.len() - line.trim_start().len())));
            }
        }
        offset += line.len();
    }
    None
}

/// The byte offset of `#include <ros/ros.h>` in C++ that uses no ROS 2 client library.
pub fn ros1_cpp_include(text: &str) -> Option<usize> {
    if text.contains("rclcpp") {
        return None;
    }
    ["#include <ros/ros.h>", "#include \"ros/ros.h\""].iter().filter_map(|p| text.find(p)).min()
}

fn is_ros1_package(p: &Package) -> bool {
    p.deps.iter().any(|d| d.name == "catkin")
}

/// Vendored third-party code and hidden directories (CI scripts) are not the package's own.
fn foreign_path(rel: &str) -> bool {
    rel.split('/').any(|c| {
        c.starts_with('.') && c.len() > 1
            || matches!(c.to_ascii_lowercase().as_str(), "third_party" | "third-party" | "thirdparty" | "3rdparty" | "vendor" | "external" | "extern")
    })
}

impl Agent for Ros1Agent {
    fn id(&self) -> &'static str {
        "ros1"
    }
    fn title(&self) -> &'static str {
        "ROS 1 leftovers (launch files, client libraries, catkin packages)"
    }
    fn inputs(&self) -> &'static [FileKind] {
        &[FileKind::Ros1, FileKind::LaunchXml, FileKind::Manifest]
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.ix.kinds.contains(&FileKind::Ros1)
            || cx.ix.packages.iter().any(is_ros1_package)
            || cx.ix.kinds.iter().enumerate().any(|(i, k)| *k == FileKind::LaunchXml && cx.ix.file(i).text.contains(" type="))
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        // A ROS 2-only package: not catkin, and not declaring ROS 1 support (REP 149).
        let ros2_pkg = |f: FileId| cx.ix.package_of(f).is_some_and(|p| !is_ros1_package(p) && !p.ros1_support);
        for f in 0..cx.ix.kinds.len() {
            if cx.ix.external_files.contains(&f) || !ros2_pkg(f) {
                continue;
            }
            let file = cx.ix.file(f);
            if foreign_path(&file.rel_path) {
                continue;
            }
            match cx.ix.kinds[f] {
                FileKind::LaunchXml => {
                    if let Some((what, loc)) = ros1_launch_construct(f, &file.text) {
                        out.push(
                            Diagnostic::new("O001", loc, format!("`{}` is a ROS 1 launch file ({what}); `ros2 launch` rejects it", file.rel_path))
                                .help(format!("Port it to ROS 2 XML (`exec=` for `type=`, `$(find-pkg-share pkg)` for `$(find pkg)`, `<param from=…>` for `<rosparam>`) or delete it. See {MIGRATE_LAUNCH}")),
                        );
                    }
                }
                FileKind::Ros1 => {
                    if let Some((m, at)) = ros1_python_import(&file.text) {
                        let end = file.text[at..].find('\n').map_or(file.text.len(), |e| at + e);
                        out.push(
                            Diagnostic::new("O002", Loc::new(f, at, end), format!("this module imports `{m}`, which ROS 2 does not have; running it fails with ModuleNotFoundError"))
                                .help("Port it to rclpy (and tf2_ros, rosbag2_py, rclpy.action), or delete it."),
                        );
                    } else if let Some(at) = ros1_cpp_include(&file.text) {
                        let end = file.text[at..].find('\n').map_or(file.text.len(), |e| at + e);
                        out.push(
                            Diagnostic::new("O002", Loc::new(f, at, end), "this file includes `ros/ros.h`, which ROS 2 does not have; it cannot compile in a ROS 2 package")
                                .help("Port it to rclcpp, or delete it."),
                        );
                    }
                }
                _ => {}
            }
        }
        // catkin packages next to ROS 2 packages.
        if cx.ix.packages.iter().any(|p| !is_ros1_package(p)) {
            for p in cx.ix.packages.iter().filter(|p| is_ros1_package(p)) {
                // colcon does not look for packages inside another package's directory.
                let nested = cx.ix.packages.iter().any(|o| o.dir != p.dir && (o.dir.is_empty() || p.dir.starts_with(&format!("{}/", o.dir))));
                if cx.ix.external_files.contains(&p.manifest) || nested || foreign_path(&p.dir) {
                    continue;
                }
                let loc = p.deps.iter().find(|d| d.name == "catkin").map_or(p.manifest_name_loc, |d| d.loc);
                out.push(
                    Diagnostic::new("O003", loc, format!("package `{}` is a catkin (ROS 1) package; colcon builds it with catkin, which fails where only ROS 2 is installed", p.name))
                        .help("Port it to ament_cmake or ament_python, or put an empty COLCON_IGNORE file in its directory."),
                );
            }
        }
    }
}

/// The first ROS 1 construct in an XML launch file: (description, location).
fn ros1_launch_construct(f: FileId, text: &str) -> Option<(&'static str, Loc)> {
    let doc = roxmltree::Document::parse(text).ok()?;
    let root = doc.root_element();
    if !root.has_tag_name("launch") {
        return None;
    }
    let loc = |n: roxmltree::Node| Loc::new(f, n.range().start, n.range().end);
    for n in root.descendants().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "node" if n.attribute("type").is_some() && n.attribute("exec").is_none() => return Some(("`<node type=…>`", loc(n))),
            "rosparam" => return Some(("`<rosparam>`", loc(n))),
            _ => {}
        }
        if n.attributes().any(|a| a.value().contains("$(find ")) {
            return Some(("`$(find pkg)`", loc(n)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::analyze_files;

    const ROS2_PKG: (&str, &str) = ("p/package.xml", "<package format=\"3\"><name>p</name><buildtool_depend>ament_cmake</buildtool_depend><export><build_type>ament_cmake</build_type></export></package>");

    #[test]
    fn python_detection() {
        assert_eq!(ros1_python_import("#!/usr/bin/env python\nimport rospy\n").map(|x| x.0), Some("rospy"));
        assert_eq!(ros1_python_import("from tf.transformations import euler_from_quaternion\n").map(|x| x.0), Some("tf"));
        assert_eq!(ros1_python_import("import tensorflow as tf\nimport rosbag2_py\nfrom rosgraph_msgs.msg import Clock\n"), None);
        assert_eq!(ros1_python_import("import rclpy\nimport rospy\n"), None);
        assert_eq!(ros1_python_import("try:\n    import rospy\nexcept ImportError:\n    pass\n"), None);
    }

    #[test]
    fn leftovers_in_a_ros2_package() {
        let d = analyze_files(&[
            ROS2_PKG,
            ("p/launch/old.launch", "<launch>\n  <node pkg=\"p\" type=\"talker\" name=\"talker\"/>\n</launch>\n"),
            ("p/launch/new.launch.xml", "<launch>\n  <node pkg=\"p\" exec=\"talker\"/>\n</launch>\n"),
            ("p/scripts/old_node.py", "#!/usr/bin/env python\nimport rospy\nrospy.init_node('x')\n"),
            ("p/src/old.cpp", "#include <ros/ros.h>\nint main() {}\n"),
            ("legacy/package.xml", "<package format=\"2\"><name>legacy</name><buildtool_depend>catkin</buildtool_depend></package>"),
            ("legacy/launch/a.launch", "<launch><node pkg=\"legacy\" type=\"x\" name=\"x\"/></launch>"),
        ]);
        let o: Vec<(&str, &str)> = d.iter().filter(|x| x.rule.starts_with('O')).map(|x| (x.rule, x.message.as_str())).collect();
        assert_eq!(o.len(), 4, "{o:#?}");
        assert!(o.iter().any(|(r, m)| *r == "O001" && m.contains("old.launch")));
        assert!(o.iter().any(|(r, m)| *r == "O002" && m.contains("`rospy`")));
        assert!(o.iter().any(|(r, m)| *r == "O002" && m.contains("ros/ros.h")));
        // The catkin package itself is reported once, its own launch file is not.
        assert!(o.iter().any(|(r, m)| *r == "O003" && m.contains("`legacy`")));
    }

    #[test]
    fn dual_vendored_and_nested_packages_are_not_leftovers() {
        let d = analyze_files(&[
            ROS2_PKG,
            // Builds for both ROS versions (REP 149 conditions): its ROS 1 files are intended.
            ("dual/package.xml", "<package format=\"3\"><name>dual</name><buildtool_depend condition=\"$ROS_VERSION == 1\">catkin</buildtool_depend><buildtool_depend condition=\"$ROS_VERSION == 2\">ament_cmake</buildtool_depend></package>"),
            ("dual/launch/driver.launch", "<launch><node pkg=\"dual\" type=\"driver\" name=\"d\"/></launch>"),
            ("dual/src/driver_ros1.cpp", "#include <ros/ros.h>\n"),
            // Vendored code and CI scripts.
            ("p/third_party/rosbag/bag.h", "#include <ros/ros.h>\n"),
            ("p/.github/scripts/ros1_check.py", "import rospy\n"),
            // A catkin package vendored inside another package: colcon never sees it.
            ("p/ros1/roscpp/package.xml", "<package format=\"2\"><name>roscpp</name><buildtool_depend>catkin</buildtool_depend></package>"),
        ]);
        let o: Vec<(&str, &str)> = d.iter().filter(|x| x.rule.starts_with('O')).map(|x| (x.rule, x.message.as_str())).collect();
        assert!(o.is_empty(), "{o:#?}");
    }

    #[test]
    fn ros1_only_workspace_is_not_a_leftover() {
        let d = analyze_files(&[
            ("legacy/package.xml", "<package format=\"2\"><name>legacy</name><buildtool_depend>catkin</buildtool_depend></package>"),
            ("legacy/scripts/n.py", "import rospy\n"),
        ]);
        assert!(!d.iter().any(|x| x.rule.starts_with('O')), "{d:#?}");
    }
}
