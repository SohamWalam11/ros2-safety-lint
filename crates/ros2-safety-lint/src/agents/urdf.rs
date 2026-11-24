//! URDF agent: the physical content of robot descriptions.
//!
//! - U001 content urdfdom rejects, so the whole model fails to load ("Failed to parse robot
//!   description"): a revolute or prismatic joint without `<limit>`, a `<limit>` without
//!   `effort` or `velocity`, an `<inertial>` without `<mass value>` or a complete `<inertia>`;
//! - U002 a moving link (the child of a revolute, continuous, prismatic, floating or planar
//!   joint) with a mass that is not positive, or an inertia that no rigid body can have
//!   (not positive definite, or principal moments violating the triangle inequality).
//!   Links on fixed joints are merged into their parent by simulators and not checked;
//! - U003 a `package://` mesh of a workspace package that does not exist, or exists only
//!   with different letter case (found on Windows and macOS, missing on Linux);
//! - U004 a revolute or prismatic joint whose `<limit>` gives neither `lower` nor `upper`:
//!   urdfdom uses 0 for both, so the joint cannot move.
//!
//! Elements that contain xacro constructs, and values that cannot be evaluated, are not
//! checked. Mesh checks run only when the package directory exists on disk, and a mesh
//! whose file name exists elsewhere in the package is not reported (install rules can move
//! it).

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::facts::{UrdfInertial, UrdfNumber};
use crate::source::Loc;
use crate::workspace::FileKind;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct UrdfAgent;

impl Agent for UrdfAgent {
    fn id(&self) -> &'static str {
        "urdf"
    }
    fn title(&self) -> &'static str {
        "Robot description physics (limits, inertia, meshes)"
    }
    fn inputs(&self) -> &'static [FileKind] {
        &[FileKind::Urdf, FileKind::Manifest]
    }
    fn relevant(&self, cx: &Context) -> bool {
        !cx.ix.robots.is_empty()
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        // A macro used several times yields the same element several times.
        let mut seen: HashSet<(Loc, String)> = HashSet::new();
        let mut push = |d: Diagnostic| {
            if !cx.ix.external_files.contains(&d.loc.file)
                && seen.insert((d.loc, d.message.clone()))
            {
                out.push(d);
            }
        };
        for robot in &cx.ix.robots {
            for j in &robot.joints {
                if j.opaque || !matches!(j.joint_type.as_str(), "revolute" | "prismatic") {
                    continue;
                }
                match &j.limit {
                    None => push(Diagnostic::new(
                        "U001",
                        j.loc,
                        format!("{} joint `{}` has no `<limit>`; urdfdom rejects the robot description (\"does not specify limits\")", j.joint_type, j.name),
                    )
                    .help("Add `<limit lower=\"…\" upper=\"…\" effort=\"…\" velocity=\"…\"/>`, or use type=\"continuous\" for an unlimited revolute joint.")),
                    Some(l) => {
                        for (attr, v) in [("effort", &l.effort), ("velocity", &l.velocity)] {
                            if v.is_none() {
                                push(Diagnostic::new(
                                    "U001",
                                    l.loc,
                                    format!("the `<limit>` of joint `{}` has no `{attr}`; urdfdom rejects the robot description (\"joint limit: no {attr}\")", j.name),
                                ));
                            }
                        }
                        if l.lower.is_none() && l.upper.is_none() {
                            push(Diagnostic::new(
                                "U004",
                                l.loc,
                                format!("the `<limit>` of {} joint `{}` sets neither `lower` nor `upper`; both default to 0, so the joint cannot move", j.joint_type, j.name),
                            ));
                        } else if let (Some(Some(lo)), Some(Some(hi))) = (l.lower, l.upper) {
                            if lo > hi {
                                push(Diagnostic::new(
                                    "U004",
                                    l.loc,
                                    format!("joint `{}` has `lower` {lo} above `upper` {hi}; no position satisfies the limits", j.name),
                                ));
                            }
                        }
                    }
                }
            }
            // Links attached by fixed joints are merged into their parent by simulators, so
            // their mass and inertia only matter for links that move.
            let moving: HashSet<&str> = robot
                .joints
                .iter()
                .filter(|j| {
                    matches!(
                        j.joint_type.as_str(),
                        "revolute" | "continuous" | "prismatic" | "floating" | "planar"
                    )
                })
                .filter_map(|j| j.child.as_deref())
                .collect();
            for i in &robot.inertials {
                inertial(i, moving.contains(i.link.as_str()), &mut push);
            }
            for m in &robot.meshes {
                if let Some(d) = mesh(cx, &m.uri, m.loc) {
                    push(d);
                }
            }
        }
    }
}

/// U001 for any link; U002 only for a link that a moving joint carries (`moving`).
fn inertial(i: &UrdfInertial, moving: bool, push: &mut impl FnMut(Diagnostic)) {
    let reject = |what: &str| {
        Diagnostic::new(
            "U001",
            i.loc,
            format!(
                "the `<inertial>` of link `{}` {what}; urdfdom rejects the robot description",
                i.link
            ),
        )
    };
    let Some(mass) = i.mass else {
        push(reject("has no `<mass value=…>`"));
        return;
    };
    let Some(inertia) = &i.inertia else {
        push(reject("has no `<inertia>`"));
        return;
    };
    const NAMES: [&str; 6] = ["ixx", "ixy", "ixz", "iyy", "iyz", "izz"];
    let missing: Vec<&str> = NAMES
        .iter()
        .zip(inertia)
        .filter(|(_, v)| v.is_none())
        .map(|(n, _)| *n)
        .collect();
    if !missing.is_empty() {
        push(
            reject(&format!("has an `<inertia>` without {}", missing.iter().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", ")))
                .help("urdfdom requires all six of ixx, ixy, ixz, iyy, iyz, izz; write 0 for products of inertia that are zero."),
        );
        return;
    }
    if !moving {
        return;
    }
    if let Some(m) = mass {
        if m <= 0.0 {
            push(Diagnostic::new(
                "U002",
                i.loc,
                format!("link `{}` has mass {m}; physics engines need a positive mass and drop or destabilize the link", i.link),
            ));
            return;
        }
    }
    let v: Vec<f64> = inertia
        .iter()
        .filter_map(|x: &UrdfNumber| x.flatten())
        .collect();
    if v.len() != 6 {
        return;
    }
    if let Some(why) = inertia_problem(v[0], v[1], v[2], v[3], v[4], v[5]) {
        push(
            Diagnostic::new("U002", i.loc, format!("the inertia of link `{}` is not physical: {why}", i.link))
                .help("Physics engines reject such a link or simulate it unstably. Compute the tensor from the geometry and mass (or use a CAD export)."),
        );
    }
}

/// Why a symmetric inertia tensor cannot belong to a rigid body, if it cannot.
fn inertia_problem(ixx: f64, ixy: f64, ixz: f64, iyy: f64, iyz: f64, izz: f64) -> Option<String> {
    let scale = ixx.abs().max(iyy.abs()).max(izz.abs());
    if scale == 0.0 {
        return Some("all moments are 0".to_string());
    }
    // Sylvester's criterion: positive definite iff all leading principal minors are > 0.
    let eps = 1e-9 * scale;
    let m2 = ixx * iyy - ixy * ixy;
    let det = ixx * (iyy * izz - iyz * iyz) - ixy * (ixy * izz - iyz * ixz)
        + ixz * (ixy * iyz - iyy * ixz);
    if ixx <= eps || m2 <= eps * scale || det <= eps * scale * scale {
        return Some("the tensor is not positive definite (a moment is 0 or negative)".to_string());
    }
    // Triangle inequality on the principal moments (checked when the tensor is diagonal).
    if ixy == 0.0 && ixz == 0.0 && iyz == 0.0 {
        let tol = 1e-3 * scale;
        for (a, b, c, n) in [
            (ixx, iyy, izz, "izz"),
            (ixx, izz, iyy, "iyy"),
            (iyy, izz, ixx, "ixx"),
        ] {
            if a + b + tol < c {
                return Some(format!(
                    "`{n}` = {c} exceeds the sum of the other two moments ({a} + {b})"
                ));
            }
        }
    }
    None
}

/// The package and path a mesh URI names inside the workspace (`package://` or `$(find)`).
fn package_path(uri: &str) -> Option<(&str, &str)> {
    let rest = uri
        .strip_prefix("package://")
        .or_else(|| uri.strip_prefix("@pkg:"))?;
    let (pkg, path) = rest.split_once('/')?;
    (!pkg.is_empty() && !path.is_empty()).then_some((pkg, path.trim_start_matches('/')))
}

/// Walks `rel` below `base` component by component with exact names. Ok(()) when found;
/// Err(Some(actual)) when found only ignoring case; Err(None) when not found.
fn find_exact(base: &Path, rel: &str) -> Result<(), Option<String>> {
    let mut dir = base.to_path_buf();
    let mut actual: Vec<String> = Vec::new();
    let mut case_differs = false;
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            dir.pop();
            actual.push("..".into());
            continue;
        }
        let names: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|_| None)?
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        let name = if names.iter().any(|n| n == part) {
            part.to_string()
        } else if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case(part)) {
            case_differs = true;
            n.clone()
        } else {
            return Err(None);
        };
        dir.push(&name);
        actual.push(name);
    }
    if case_differs {
        Err(Some(actual.join("/")))
    } else {
        Ok(())
    }
}

/// True if a file with this name (any case) exists anywhere below `dir`.
fn name_exists_below(dir: &Path, name: &str, budget: &mut usize) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in rd.flatten() {
        if *budget == 0 {
            return true; // too large to search: assume it exists
        }
        *budget -= 1;
        let p: PathBuf = e.path();
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return true;
        }
        if p.is_dir()
            && !e.file_name().to_string_lossy().starts_with('.')
            && name_exists_below(&p, name, budget)
        {
            return true;
        }
    }
    false
}

fn mesh(cx: &Context, uri: &str, loc: Loc) -> Option<Diagnostic> {
    if uri.contains("${") || uri.contains("$(") {
        return None;
    }
    let (pkg, path) = package_path(uri)?;
    let p = cx.ix.package_by_name(pkg)?;
    let base = cx.ix.root.join(&p.dir);
    if !base.is_dir() {
        return None;
    }
    match find_exact(&base, path) {
        Ok(()) => None,
        Err(Some(actual)) => Some(
            Diagnostic::new(
                "U003",
                loc,
                format!("mesh `{uri}` exists only as `{actual}` (different letter case); it loads on Windows and macOS but not on Linux"),
            )
            .safe_fix(format!("Write `package://{pkg}/{actual}`"), loc.file, loc.start, loc.end, format!("package://{pkg}/{actual}")),
        ),
        Err(None) => {
            let name = path.rsplit('/').next().unwrap_or(path);
            let mut budget = 20_000;
            (!name_exists_below(&base, name, &mut budget)).then(|| {
                Diagnostic::new(
                    "U003",
                    loc,
                    format!("mesh `{uri}` does not exist in package `{pkg}`; RViz and Gazebo show the link without it (\"Could not load resource\")"),
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::analyze_files;

    fn urdf(body: &str) -> Vec<Diagnostic> {
        let text = format!("<robot name=\"r\"><link name=\"base\"/>{body}</robot>");
        analyze_files(&[
            (
                "r/package.xml",
                "<package format=\"3\"><name>r</name></package>",
            ),
            ("r/urdf/r.urdf", &text),
        ])
    }

    fn rules(d: &[Diagnostic]) -> Vec<&str> {
        let mut r: Vec<&str> = d
            .iter()
            .filter(|x| x.rule.starts_with('U'))
            .map(|x| x.rule)
            .collect();
        r.sort();
        r
    }

    #[test]
    fn limits() {
        let j = |t: &str, lim: &str| {
            format!("<link name=\"a\"/><joint name=\"j\" type=\"{t}\"><parent link=\"base\"/><child link=\"a\"/>{lim}</joint>")
        };
        assert_eq!(rules(&urdf(&j("revolute", ""))), ["U001"]);
        assert_eq!(
            rules(&urdf(&j(
                "prismatic",
                "<limit lower=\"0\" upper=\"1\" effort=\"1\"/>"
            ))),
            ["U001"]
        );
        assert_eq!(
            rules(&urdf(&j(
                "revolute",
                "<limit effort=\"1\" velocity=\"1\"/>"
            ))),
            ["U004"]
        );
        assert_eq!(
            rules(&urdf(&j(
                "revolute",
                "<limit lower=\"1\" upper=\"-1\" effort=\"1\" velocity=\"1\"/>"
            ))),
            ["U004"]
        );
        assert!(rules(&urdf(&j(
            "revolute",
            "<limit lower=\"-1.57\" upper=\"1.57\" effort=\"1\" velocity=\"1\"/>"
        )))
        .is_empty());
        assert!(rules(&urdf(&j("continuous", ""))).is_empty());
        assert!(rules(&urdf(&j("fixed", ""))).is_empty());
    }

    #[test]
    fn inertia() {
        let l = |inertial: &str| {
            format!("<link name=\"a\"><inertial>{inertial}</inertial></link><joint name=\"ja\" type=\"continuous\"><parent link=\"base\"/><child link=\"a\"/></joint>")
        };
        let full = "ixx=\"0.1\" ixy=\"0\" ixz=\"0\" iyy=\"0.1\" iyz=\"0\" izz=\"0.1\"";
        assert!(rules(&urdf(&l(&format!("<mass value=\"1\"/><inertia {full}/>")))).is_empty());
        assert_eq!(
            rules(&urdf(&l(
                "<mass value=\"1\"/><inertia ixx=\"0.1\" iyy=\"0.1\" izz=\"0.1\"/>"
            ))),
            ["U001"]
        );
        assert_eq!(rules(&urdf(&l(&format!("<inertia {full}/>")))), ["U001"]);
        assert_eq!(rules(&urdf(&l("<mass value=\"1\"/>"))), ["U001"]);
        assert_eq!(
            rules(&urdf(&l(&format!("<mass value=\"0\"/><inertia {full}/>")))),
            ["U002"]
        );
        assert_eq!(rules(&urdf(&l("<mass value=\"1\"/><inertia ixx=\"0\" ixy=\"0\" ixz=\"0\" iyy=\"0\" iyz=\"0\" izz=\"0\"/>"))), ["U002"]);
        assert_eq!(rules(&urdf(&l("<mass value=\"1\"/><inertia ixx=\"0.1\" ixy=\"0\" ixz=\"0\" iyy=\"0.1\" iyz=\"0\" izz=\"0.5\"/>"))), ["U002"]);
        // A frame link on a fixed joint is merged into its parent: its mass does not matter,
        // but urdfdom still rejects an incomplete <inertia>.
        let fixed = |inertial: &str| {
            format!("<link name=\"f\"><inertial>{inertial}</inertial></link><joint name=\"jf\" type=\"fixed\"><parent link=\"base\"/><child link=\"f\"/></joint>")
        };
        assert!(rules(&urdf(&fixed(&format!(
            "<mass value=\"0\"/><inertia {full}/>"
        ))))
        .is_empty());
        assert_eq!(
            rules(&urdf(&fixed("<mass value=\"0\"/><inertia ixx=\"0\"/>"))),
            ["U001"]
        );
        // Values that cannot be evaluated are not judged.
        assert!(rules(&urdf(&l("<mass value=\"${m}\"/><inertia ixx=\"${i}\" ixy=\"0\" ixz=\"0\" iyy=\"0.1\" iyz=\"0\" izz=\"0.1\"/>"))).is_empty());
    }

    #[test]
    fn inertia_math() {
        // A thin rod along z: izz is 0, so the tensor is singular.
        assert!(inertia_problem(1.0, 0.0, 0.0, 1.0, 0.0, 0.0).is_some());
        // A flat plate: izz = ixx + iyy exactly.
        assert!(inertia_problem(1.0, 0.0, 0.0, 2.0, 0.0, 3.0).is_none());
        // A rotated (non-diagonal) valid tensor.
        assert!(inertia_problem(2.0, 0.5, 0.0, 2.0, 0.0, 3.0).is_none());
        assert!(inertia_problem(1.0, 2.0, 0.0, 1.0, 0.0, 1.0).is_some());
    }

    #[test]
    fn meshes_on_disk() {
        let m = |f: &str| format!("<visual><geometry><mesh filename=\"{f}\"/></geometry></visual>");
        let text = format!(
            "<robot name=\"b\"><link name=\"a\">{}{}{}{}{}</link></robot>",
            m("package://bot/meshes/visual/arm.dae"),
            m("package://bot/meshes/visual/base.stl"),
            m("package://bot/meshes/visual/gone.stl"),
            m("package://bot/meshes/arm.dae"),
            m("package://other_pkg/meshes/x.stl"),
        );
        let d = analyze_files(&[
            (
                "bot/package.xml",
                "<package format=\"3\"><name>bot</name></package>",
            ),
            ("bot/meshes/visual/Base.STL", "solid"),
            ("bot/meshes/visual/arm.dae", "<x/>"),
            ("bot/urdf/b.urdf", &text),
        ]);
        let u: Vec<&Diagnostic> = d.iter().filter(|x| x.rule == "U003").collect();
        assert_eq!(u.len(), 2, "{u:#?}");
        assert!(
            u.iter()
                .any(|x| x.message.contains("meshes/visual/Base.STL") && x.fix.is_some()),
            "{u:#?}"
        );
        assert!(u.iter().any(|x| x.message.contains("gone.stl")), "{u:#?}");
    }
}
