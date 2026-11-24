//! `rosfix init`: writes a starting configuration, a CI workflow and a pre-commit entry.
//!
//! Existing files are never changed unless `force` is set, and an existing
//! `.pre-commit-config.yaml` is never edited (merging YAML by hand loses comments); the
//! entry to add is printed instead.

use std::path::{Path, PathBuf};

pub struct InitOptions {
    pub distro: Option<String>,
    pub ci: bool,
    pub pre_commit: bool,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Created,
    Replaced,
    /// Kept as it is; the string says why.
    Kept(String),
}

pub struct Outcome {
    pub path: PathBuf,
    pub action: Action,
}

pub fn config_toml(distro: Option<&str>) -> String {
    let distro_line = match distro {
        Some(d) => format!("distro = \"{d}\"\n"),
        None => "# distro = \"jazzy\"                  # humble, jazzy, kilted or rolling\n".to_string(),
    };
    format!(
        "# rosfix configuration. Every key is optional; see `rosfix docs config`.\n\
         {distro_line}\
         \n\
         # Exit with code 1 when findings at or above this severity remain.\n\
         fail_on = \"warning\"\n\
         \n\
         # Paths to skip (globs relative to this file).\n\
         ignore = []\n\
         \n\
         # Topics provided outside this workspace (drivers, simulators, other robots).\n\
         external_topics = []\n\
         \n\
         [rules]\n\
         # P003 = \"off\"                        # or \"info\", \"warning\", \"error\"\n"
    )
}

pub fn github_workflow(distro: Option<&str>) -> String {
    let distro_input = distro
        .map(|d| format!("          distro: {d}\n"))
        .unwrap_or_default();
    format!(
        "name: rosfix\n\
         \n\
         on:\n\
         \x20 push:\n\
         \x20 pull_request:\n\
         \n\
         permissions:\n\
         \x20 contents: read\n\
         \x20 security-events: write\n\
         \n\
         jobs:\n\
         \x20 rosfix:\n\
         \x20   runs-on: ubuntu-latest\n\
         \x20   steps:\n\
         \x20     - uses: actions/checkout@v4\n\
         \x20     - uses: SohamWalam11/ros2-safety-lint@v1\n\
         \x20       with:\n\
         \x20         path: .\n\
         \x20         fail-on: error\n\
         {distro_input}"
    )
}

pub const PRE_COMMIT: &str = "repos:\n  - repo: https://github.com/SohamWalam11/ros2-safety-lint\n    rev: v0.2.3\n    hooks:\n      - id: rosfix\n";

fn write(root: &Path, rel: &str, content: &str, o: &InitOptions) -> std::io::Result<Outcome> {
    let path = root.join(rel);
    let exists = path.exists();
    let action = if exists && !o.force {
        Action::Kept("exists; use --force to replace it".into())
    } else if exists {
        Action::Replaced
    } else {
        Action::Created
    };
    if !o.dry_run && matches!(action, Action::Created | Action::Replaced) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, content)?;
    }
    Ok(Outcome { path, action })
}

pub fn run(root: &Path, o: &InitOptions) -> std::io::Result<Vec<Outcome>> {
    let distro = o.distro.as_deref();
    let mut out = vec![write(root, "rosfix.toml", &config_toml(distro), o)?];
    if o.ci {
        out.push(write(root, ".github/workflows/rosfix.yml", &github_workflow(distro), o)?);
    }
    if o.pre_commit {
        let path = root.join(".pre-commit-config.yaml");
        if path.exists() {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let why = if text.contains("ros2-safety-lint") {
                "already runs rosfix".to_string()
            } else {
                format!("exists; add this entry under `repos:`\n{}", &PRE_COMMIT["repos:\n".len()..])
            };
            out.push(Outcome {
                path,
                action: Action::Kept(why),
            });
        } else {
            out.push(write(root, ".pre-commit-config.yaml", PRE_COMMIT, o)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_config_is_valid() {
        for d in [None, Some("jazzy")] {
            let raw = toml::from_str(&config_toml(d)).expect("valid TOML");
            let c = crate::config::Config::from_raw(raw, None).expect("valid rosfix config");
            assert_eq!(c.distro.as_deref(), d);
        }
    }

    #[test]
    fn never_overwrites_without_force() {
        let dir = std::env::temp_dir().join(format!("rosfix-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rosfix.toml"), "fail_on = \"error\"\n").unwrap();
        std::fs::write(dir.join(".pre-commit-config.yaml"), "repos: []\n").unwrap();
        let mut o = InitOptions {
            distro: Some("humble".into()),
            ci: true,
            pre_commit: true,
            force: false,
            dry_run: false,
        };
        let r = run(&dir, &o).unwrap();
        assert!(matches!(r[0].action, Action::Kept(_)));
        assert_eq!(r[1].action, Action::Created);
        assert!(matches!(r[2].action, Action::Kept(_)));
        assert_eq!(std::fs::read_to_string(dir.join("rosfix.toml")).unwrap(), "fail_on = \"error\"\n");
        assert_eq!(std::fs::read_to_string(dir.join(".pre-commit-config.yaml")).unwrap(), "repos: []\n");
        assert!(std::fs::read_to_string(dir.join(".github/workflows/rosfix.yml"))
            .unwrap()
            .contains("distro: humble"));
        o.force = true;
        let r = run(&dir, &o).unwrap();
        assert_eq!(r[0].action, Action::Replaced);
        // --force never touches an existing pre-commit file.
        assert!(matches!(r[2].action, Action::Kept(_)));
        assert_eq!(std::fs::read_to_string(dir.join(".pre-commit-config.yaml")).unwrap(), "repos: []\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("rosfix-init-dry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let o = InitOptions {
            distro: None,
            ci: true,
            pre_commit: true,
            force: false,
            dry_run: true,
        };
        let r = run(&dir, &o).unwrap();
        assert!(r.iter().all(|x| x.action == Action::Created));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
