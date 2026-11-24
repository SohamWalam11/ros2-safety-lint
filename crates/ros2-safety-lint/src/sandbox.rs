//! `--verify-build`: differential build check of proposed fixes in a throwaway copy.
//!
//! The workspace is copied twice into a temporary directory — unchanged and with the
//! fixes applied — and both copies are built with `colcon build`. The fixes pass if the
//! patched copy builds whenever the original does. The user's tree is never built or
//! modified. With Docker available (and a known distro) the build runs in a
//! `ros:<distro>-ros-base` container with networking disabled; otherwise a local
//! `colcon` (from the sourced ROS environment) is used in the temporary copy.

use crate::fix::FixReport;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SKIP: &[&str] = &[
    ".git",
    "build",
    "install",
    "log",
    "target",
    ".rosfix",
    "node_modules",
    "__pycache__",
];
const BUILD_TIMEOUT: Duration = Duration::from_secs(45 * 60);

#[derive(Debug)]
pub enum Outcome {
    /// Both copies build.
    Passed,
    /// The original does not build either: nothing can be concluded.
    Inconclusive(String),
    /// The original builds but the patched copy does not.
    Failed(String),
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    for e in walkdir::WalkDir::new(src).into_iter().filter_entry(|e| {
        e.depth() == 0 || !SKIP.contains(&e.file_name().to_string_lossy().as_ref())
    }) {
        let e = e.map_err(std::io::Error::other)?;
        let rel = e.path().strip_prefix(src).unwrap_or(e.path());
        let target = dst.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if e.file_type().is_file() {
            std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}

fn on_path(exe: &str) -> bool {
    Command::new(exe)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Runs a command with a deadline; returns (success, last lines of output).
fn run(mut cmd: Command) -> (bool, String) {
    let tmp_log = std::env::temp_dir().join(format!(
        "rosfix-build-{}-{}.log",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let Ok(log) = std::fs::File::create(&tmp_log) else {
        return (false, "cannot create log".into());
    };
    let Ok(log2) = log.try_clone() else {
        return (false, "cannot create log".into());
    };
    let child = cmd.stdout(log).stderr(log2).spawn();
    let Ok(mut child) = child else {
        return (false, "could not start the build command".into());
    };
    let start = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.success(),
            Ok(None) if start.elapsed() > BUILD_TIMEOUT => {
                let _ = child.kill();
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(500)),
            Err(_) => break false,
        }
    };
    let text = std::fs::read_to_string(&tmp_log).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp_log);
    let tail: Vec<&str> = text.lines().rev().take(25).collect();
    (ok, tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

fn build(dir: &Path, distro: Option<&str>) -> Result<(bool, String), String> {
    if let (true, Some(d)) = (on_path("docker"), distro) {
        let mut c = Command::new("docker");
        c.args(["run", "--rm", "--network", "none", "-v"])
            .arg(format!("{}:/ws", dir.display()))
            .args(["-w", "/ws", &format!("ros:{d}-ros-base"), "bash", "-c"])
            .arg(format!(
                "source /opt/ros/{d}/setup.bash && colcon build --event-handlers console_cohesion-"
            ));
        return Ok(run(c));
    }
    if on_path("colcon") {
        let mut c = Command::new("colcon");
        c.current_dir(dir)
            .args(["build", "--event-handlers", "console_cohesion-"]);
        return Ok(run(c));
    }
    Err("neither Docker (with a known --distro) nor colcon is available for --verify-build".into())
}

/// Builds the original and the patched workspace in temporary copies.
pub fn verify_build(
    root: &Path,
    report: &FixReport,
    distro: Option<&str>,
) -> Result<Outcome, String> {
    // Fail before copying a possibly large workspace twice.
    if !(on_path("docker") && distro.is_some()) && !on_path("colcon") {
        return Err(
            "neither Docker (with a known --distro) nor colcon is available for --verify-build"
                .into(),
        );
    }
    let base: PathBuf = std::env::temp_dir().join(format!("rosfix-verify-{}", std::process::id()));
    let (orig, patched) = (base.join("original"), base.join("patched"));
    let _ = std::fs::remove_dir_all(&base);
    copy_tree(root, &orig).map_err(|e| format!("copying the workspace: {e}"))?;
    copy_tree(root, &patched).map_err(|e| format!("copying the workspace: {e}"))?;
    for (abs, (_, new_text)) in &report.files {
        let rel = abs
            .strip_prefix(root)
            .map_err(|_| format!("{} is outside the workspace", abs.display()))?;
        std::fs::write(patched.join(rel), new_text).map_err(|e| e.to_string())?;
    }
    let result = (|| {
        let (orig_ok, orig_log) = build(&orig, distro)?;
        if !orig_ok {
            return Ok(Outcome::Inconclusive(orig_log));
        }
        let (patched_ok, patched_log) = build(&patched, distro)?;
        Ok(if patched_ok {
            Outcome::Passed
        } else {
            Outcome::Failed(patched_log)
        })
    })();
    let _ = std::fs::remove_dir_all(&base);
    result
}
