//! Sim-time agent: consistent `use_sim_time` across a launch graph.

use super::{Agent, Context};
use crate::catalog::SIM_PACKAGES;
use crate::diag::Diagnostic;
use crate::facts::ParamValue;
use crate::resolve::{NodeInstance, RootGraph};

pub struct SimTimeAgent;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sim {
    True,
    False,
    /// Not set anywhere: ROS default is false.
    Unset,
    Unknown,
}

fn sim_value(n: &NodeInstance) -> Sim {
    match n.param("use_sim_time") {
        None => {
            if n.param_sources_complete {
                Sim::Unset
            } else {
                Sim::Unknown
            }
        }
        Some(b) => match b.value.as_ref().and_then(ParamValue::as_bool).or_else(|| {
            b.raw
                .as_deref()
                .and_then(|r| ParamValue::Str(r.into()).as_bool())
        }) {
            Some(true) => Sim::True,
            Some(false) => Sim::False,
            None => Sim::Unknown,
        },
    }
}

fn is_sim_node(n: &NodeInstance) -> bool {
    n.pkg.as_deref().is_some_and(|p| SIM_PACKAGES.contains(&p))
}

fn has_simulator(root: &RootGraph) -> bool {
    root.nodes.iter().any(is_sim_node)
        || root.external_includes.iter().any(|p| {
            ["gazebo", "gz_sim", "ign_gazebo", "webots", "isaac"]
                .iter()
                .any(|s| p.contains(s))
        })
}

fn has_clock_source(root: &RootGraph) -> bool {
    has_simulator(root)
        || root.nodes.iter().any(|n| {
            n.pkg.as_deref().is_some_and(|p| p.contains("rosbag2"))
                || n.exec.as_deref().is_some_and(|e| e.contains("bag"))
        })
}

impl Agent for SimTimeAgent {
    fn id(&self) -> &'static str {
        "simtime"
    }
    fn title(&self) -> &'static str {
        "Simulated time consistency"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        super::GRAPH_INPUTS
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.graph
            .roots
            .iter()
            .any(|r| r.nodes.len() > 1 || has_simulator(r))
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        for root in &cx.graph.roots {
            let nodes: Vec<(&NodeInstance, Sim)> = root
                .nodes
                .iter()
                .filter(|n| !n.conditional && !is_sim_node(n))
                .map(|n| (n, sim_value(n)))
                .collect();
            let true_nodes: Vec<String> = nodes
                .iter()
                .filter(|(_, v)| *v == Sim::True)
                .map(|(n, _)| n.display_name())
                .collect();
            if has_simulator(root) {
                for (n, v) in &nodes {
                    if matches!(v, Sim::False | Sim::Unset) {
                        let how = if *v == Sim::False {
                            "sets `use_sim_time: false`"
                        } else {
                            "does not set `use_sim_time`"
                        };
                        out.push(
                            Diagnostic::new("S002", n.decl.loc, format!("a simulator is launched, but node `{}` {how}, so it runs on wall-clock time", n.display_name()))
                                .help("Pass `use_sim_time: true` (e.g. via a launch argument or `SetParameter(name='use_sim_time', value=True)`)."),
                        );
                    }
                }
                continue;
            }
            if true_nodes.is_empty() {
                continue;
            }
            for (n, v) in &nodes {
                if *v == Sim::False {
                    out.push(
                        Diagnostic::new(
                            "S001",
                            n.decl.loc,
                            format!("node `{}` uses wall-clock time while {} use simulated time", n.display_name(), list(&true_nodes)),
                        )
                        .help("Mixed clocks cause tf `extrapolation` errors. Use one `use_sim_time` launch argument for every node."),
                    );
                }
            }
            if !root.partial
                && !has_clock_source(root)
                && nodes.iter().all(|(_, v)| *v != Sim::Unknown)
            {
                if let Some((n, _)) = nodes.iter().find(|(_, v)| *v == Sim::True) {
                    out.push(Diagnostic::new(
                        "S003",
                        n.decl.loc,
                        format!("{} use simulated time, but nothing in this launch graph publishes `/clock`", list(&true_nodes)),
                    ));
                }
            }
        }
    }
}

fn list(v: &[String]) -> String {
    let shown: Vec<String> = v.iter().take(3).map(|s| format!("`{s}`")).collect();
    if v.len() > 3 {
        format!("{} and {} more", shown.join(", "), v.len() - 3)
    } else {
        shown.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn mixed_and_simulator() {
        let d = analyze_files(&[(
            "p/launch/a.launch.py",
            "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='a', executable='x', name='x', parameters=[{'use_sim_time': True}]),\n        Node(package='b', executable='y', name='y', parameters=[{'use_sim_time': False}]),\n    ])\n",
        ), ("p/package.xml", "<package format=\"3\"><name>p</name></package>")]);
        assert!(d.iter().any(|x| x.rule == "S001"), "{d:#?}");
        let d = analyze_files(&[(
            "p/launch/b.launch.py",
            "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([\n        Node(package='ros_gz_bridge', executable='parameter_bridge'),\n        Node(package='b', executable='y', name='y'),\n    ])\n",
        ), ("p/package.xml", "<package format=\"3\"><name>p</name></package>")]);
        assert!(d.iter().any(|x| x.rule == "S002"), "{d:#?}");
    }

    /// rmf_demos fleet_adapter.launch.xml: an includer passes `use_sim_time:=false`, so the
    /// `unless` branch runs on wall-clock time beside a sim-time node (S001), while the `if`
    /// branch is not taken and must not be reported.
    #[test]
    fn if_unless_branches_follow_the_passed_argument() {
        let inner = r#"<launch>
  <arg name="use_sim_time" default="true"/>
  <group>
    <group if="$(var use_sim_time)">
      <node pkg="p" exec="a" name="in_if"><param name="use_sim_time" value="$(var use_sim_time)"/></node>
    </group>
    <group unless="$(var use_sim_time)">
      <node pkg="p" exec="a" name="in_unless"><param name="use_sim_time" value="$(var use_sim_time)"/></node>
    </group>
  </group>
</launch>"#;
        let top = r#"<launch>
  <node pkg="p" exec="b" name="sim_node"><param name="use_sim_time" value="true"/></node>
  <include file="$(find-pkg-share p)/launch/inner.launch.xml">
    <arg name="use_sim_time" value="false"/>
  </include>
</launch>"#;
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name></package>"),
            ("p/CMakeLists.txt", "project(p)\nadd_executable(a src/a.cpp)\nadd_executable(b src/b.cpp)\ninstall(TARGETS a b DESTINATION lib/${PROJECT_NAME})\ninstall(DIRECTORY launch DESTINATION share/${PROJECT_NAME})\n"),
            ("p/src/a.cpp", "int main() { auto n = std::make_shared<rclcpp::Node>(\"a\"); }\n"),
            ("p/src/b.cpp", "int main() { auto n = std::make_shared<rclcpp::Node>(\"b\"); }\n"),
            ("p/launch/inner.launch.xml", inner),
            ("p/launch/top.launch.xml", top),
        ]);
        let s001: Vec<&str> = d
            .iter()
            .filter(|x| x.rule == "S001")
            .map(|x| x.message.as_str())
            .collect();
        assert!(s001.iter().any(|m| m.contains("in_unless")), "{s001:#?}");
        assert!(s001.iter().all(|m| !m.contains("in_if")), "{s001:#?}");
    }
}
