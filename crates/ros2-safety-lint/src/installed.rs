//! The user's installed ROS 2 environment (`AMENT_PREFIX_PATH`), read through the ament
//! resource index. This lets rosfix understand packages that are not in the workspace:
//! which executables they install, which plugins and components they export, and their
//! launch/config files (loaded on demand when a workspace launch file includes them).
//!
//! Layout used (ament_cmake `resource_index.md`, pluginlib cmake hooks):
//! - `<prefix>/share/ament_index/resource_index/packages/<pkg>` — one marker per package
//! - `<prefix>/share/<pkg>/` — package share directory (package.xml, launch, config…)
//! - `<prefix>/lib/<pkg>/` — executables found by `ros2 run` / launch
//! - `<prefix>/share/ament_index/resource_index/<category>__pluginlib__plugin/<pkg>` —
//!   lines `share/<pkg>/<file>.xml` naming plugin description files
//! - `<prefix>/share/ament_index/resource_index/rclcpp_components/<pkg>` —
//!   lines `<ns::Class>;<library path>`

use crate::extract::package::{parse_manifest, parse_plugin_xml};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct InstalledPackage {
    pub name: String,
    pub prefix: PathBuf,
    pub share: PathBuf,
    pub executables: Vec<String>,
    /// Plugin lookup names and types exported through pluginlib.
    pub plugin_classes: Vec<String>,
    /// rclcpp components registered by this package (`ns::Class`).
    pub components: Vec<String>,
    pub deps: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct InstalledEnv {
    pub prefixes: Vec<PathBuf>,
    pub packages: HashMap<String, InstalledPackage>,
    /// ROS distribution (`ROS_DISTRO`, or inferred from `/opt/ros/<distro>`).
    pub distro: Option<String>,
}

impl InstalledEnv {
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&InstalledPackage> {
        self.packages.get(name)
    }

    /// Resolves `share/<pkg>/<sub>` of an installed package to an absolute path.
    pub fn share_path(&self, pkg: &str, sub: &str) -> Option<PathBuf> {
        self.packages.get(pkg).map(|p| p.share.join(sub))
    }

    pub fn all_plugin_classes(&self) -> impl Iterator<Item = &str> {
        self.packages
            .values()
            .flat_map(|p| p.plugin_classes.iter().map(|s| s.as_str()))
    }
}

fn resource_dir(prefix: &Path) -> PathBuf {
    prefix
        .join("share")
        .join("ament_index")
        .join("resource_index")
}

fn file_names(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();
    v.sort();
    v
}

/// Infers the distro name from a prefix such as `/opt/ros/jazzy`.
fn distro_from_prefix(p: &Path) -> Option<String> {
    let s = p.to_string_lossy().replace('\\', "/");
    let i = s.find("/opt/ros/")?;
    let rest = &s[i + "/opt/ros/".len()..];
    let d = rest.split('/').next()?;
    (!d.is_empty()).then(|| d.to_string())
}

/// Reads all prefixes. Earlier prefixes win, matching ament's lookup order.
pub fn discover(prefixes: &[PathBuf], ros_distro: Option<String>) -> InstalledEnv {
    let mut env = InstalledEnv {
        prefixes: prefixes.to_vec(),
        ..Default::default()
    };
    for prefix in prefixes {
        let rdir = resource_dir(prefix);
        for name in file_names(&rdir.join("packages")) {
            if env.packages.contains_key(&name) {
                continue;
            }
            let share = prefix.join("share").join(&name);
            let mut executables: Vec<String> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(prefix.join("lib").join(&name)) {
                for e in rd.filter_map(Result::ok) {
                    if e.file_type()
                        .map(|t| t.is_file() || t.is_symlink())
                        .unwrap_or(false)
                    {
                        if let Some(n) = e.file_name().to_str() {
                            let n = n.strip_suffix(".exe").unwrap_or(n);
                            if !n.ends_with(".so") && !n.ends_with(".dll") && !n.ends_with(".dylib")
                            {
                                executables.push(n.to_string());
                            }
                        }
                    }
                }
            }
            executables.sort();
            let deps = std::fs::read_to_string(share.join("package.xml"))
                .ok()
                .and_then(|t| parse_manifest(0, &t).ok())
                .map(|m| m.deps.into_iter().map(|d| d.name).collect())
                .unwrap_or_default();
            env.packages.insert(
                name.clone(),
                InstalledPackage {
                    name,
                    prefix: prefix.clone(),
                    share,
                    executables,
                    deps,
                    ..Default::default()
                },
            );
        }
        // Plugins: resource types ending in `__pluginlib__plugin`.
        for rtype in file_names(&rdir) {
            if rtype.ends_with("__pluginlib__plugin") {
                for pkg in file_names(&rdir.join(&rtype)) {
                    let Ok(marker) = std::fs::read_to_string(rdir.join(&rtype).join(&pkg)) else {
                        continue;
                    };
                    let mut classes = Vec::new();
                    for line in marker.lines().map(str::trim).filter(|l| !l.is_empty()) {
                        if let Ok(xml) = std::fs::read_to_string(prefix.join(line)) {
                            classes.extend(parse_plugin_xml(&xml));
                        }
                    }
                    if let Some(p) = env.packages.get_mut(&pkg) {
                        if p.prefix == *prefix {
                            p.plugin_classes.extend(classes);
                        }
                    }
                }
            }
        }
        // rclcpp components.
        for pkg in file_names(&rdir.join("rclcpp_components")) {
            let Ok(marker) = std::fs::read_to_string(rdir.join("rclcpp_components").join(&pkg))
            else {
                continue;
            };
            if let Some(p) = env.packages.get_mut(&pkg) {
                if p.prefix == *prefix {
                    p.components.extend(
                        marker
                            .lines()
                            .filter_map(|l| l.split(';').next())
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                    );
                }
            }
        }
    }
    env.distro = ros_distro
        .filter(|d| !d.is_empty())
        .or_else(|| prefixes.iter().find_map(|p| distro_from_prefix(p)));
    env
}

/// Prefixes from `AMENT_PREFIX_PATH`.
pub fn prefixes_from_env() -> Vec<PathBuf> {
    std::env::var_os("AMENT_PREFIX_PATH")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a fake installed prefix with two packages.
    pub fn fake_prefix(root: &Path) -> PathBuf {
        let p = root.join("opt").join("ros").join("jazzy");
        let w = |rel: &str, text: &str| {
            let f = p.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        };
        w(
            "share/ament_index/resource_index/packages/nav2_controller",
            "",
        );
        w("share/ament_index/resource_index/packages/nav2_bringup", "");
        w(
            "share/nav2_controller/package.xml",
            "<package format=\"3\"><name>nav2_controller</name><depend>rclcpp</depend></package>",
        );
        w(
            "share/nav2_bringup/package.xml",
            "<package format=\"3\"><name>nav2_bringup</name></package>",
        );
        w("lib/nav2_controller/controller_server", "bin");
        w("lib/nav2_controller/libplugin.so", "so");
        w(
            "share/ament_index/resource_index/nav2_core__pluginlib__plugin/nav2_controller",
            "share/nav2_controller/plugins.xml\n",
        );
        w("share/nav2_controller/plugins.xml", "<class_libraries><library path=\"x\"><class type=\"nav2_controller::SimpleGoalChecker\" base_class_type=\"nav2_core::GoalChecker\"/></library></class_libraries>");
        w(
            "share/ament_index/resource_index/rclcpp_components/nav2_controller",
            "nav2_controller::ControllerServer;lib/libcontroller_server_core.so\n",
        );
        w(
            "share/nav2_bringup/launch/bringup.launch.py",
            "from launch import LaunchDescription\nfrom launch_ros.actions import Node\ndef generate_launch_description():\n    return LaunchDescription([Node(package='nav2_controller', executable='controller_server', name='controller_server')])\n",
        );
        p
    }

    #[test]
    fn reads_resource_index() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake_prefix(dir.path());
        let env = discover(&[p], None);
        let c = env.get("nav2_controller").unwrap();
        assert_eq!(c.executables, vec!["controller_server"]);
        assert_eq!(c.plugin_classes, vec!["nav2_controller::SimpleGoalChecker"]);
        assert_eq!(c.components, vec!["nav2_controller::ControllerServer"]);
        assert_eq!(c.deps, vec!["rclcpp"]);
        assert_eq!(env.distro.as_deref(), Some("jazzy"));
    }
}
