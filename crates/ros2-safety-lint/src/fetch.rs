//! Local knowledge packs built from upstream sources (`--online`).
//!
//! The embedded packs cover the nodes of a fixed set of repositories. When a workspace launches
//! a package that is neither in the workspace nor in the pack of its distribution, `--online`
//! looks the package up in the distribution's public rosdistro index, downloads the source of
//! its repository at the distribution's branch, runs rosfix's own extractors over it once and
//! stores the result as a *local pack* in the cache. Later runs use the local pack offline.
//!
//! Privacy and isolation, as for the reference lookup in `assist::online`:
//! - The analysing process never opens a network connection; downloads go through the
//!   `rosfix __fetch` helper and its HTTPS allowlist (the rosdistro index on
//!   raw.githubusercontent.com and source archives on codeload.github.com).
//! - Package names are matched against the downloaded index **locally**. Only repositories
//!   listed in the public index are requested, so names of private packages never leave the
//!   machine. The requests do show GitHub which public repositories the workspace uses.
//! - Downloaded files are never executed: archives are unpacked with path checks and size
//!   limits, only source, build and manifest files are kept, and the extractors parse them
//!   statically. The unpacked sources are deleted once the pack is built.

use crate::pack;
use crate::workspace::Index;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A source repository of a distribution, from rosdistro's `distribution.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    /// Branch (or tag) of the source entry for this distribution.
    pub version: String,
}

impl Repo {
    /// File-system key, also the name of the local pack file.
    pub fn key(&self) -> String {
        format!("{}__{}", self.owner, self.name)
    }

    pub fn tarball_url(&self) -> String {
        format!(
            "https://codeload.github.com/{}/{}/tar.gz/{}",
            self.owner, self.name, self.version
        )
    }

    pub fn display(&self) -> String {
        format!("{}/{}@{}", self.owner, self.name, self.version)
    }
}

fn safe_token(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `https://github.com/<owner>/<repo>(.git)` → (owner, repo).
fn github_parts(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut it = rest.trim_end_matches('/').split('/');
    let owner = it.next()?;
    let repo = it.next()?.trim_end_matches(".git");
    if it.next().is_some() || !safe_token(owner) || !safe_token(repo) {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Package name → source repository, for the released packages of a `distribution.yaml`.
/// Repositories that are not on GitHub, or whose names do not look like plain identifiers,
/// are left out.
pub fn parse_distribution(yaml: &str) -> BTreeMap<String, Repo> {
    use yaml_rust2::{Yaml, YamlLoader};
    let mut out = BTreeMap::new();
    let Ok(docs) = YamlLoader::load_from_str(yaml) else {
        return out;
    };
    let Some(repos) = docs.first().and_then(|d| d["repositories"].as_hash()) else {
        return out;
    };
    for (k, v) in repos {
        let Some(repo_name) = k.as_str() else {
            continue;
        };
        let release = &v["release"];
        if release.is_badvalue() {
            continue; // not released: not installable from binaries
        }
        let src = if v["source"]["url"].as_str().is_some() {
            &v["source"]
        } else {
            &v["doc"]
        };
        let (Some(url), Some(version)) = (src["url"].as_str(), src["version"].as_str()) else {
            continue;
        };
        let Some((owner, name)) = github_parts(url) else {
            continue;
        };
        if !version.split('/').all(safe_token) {
            continue;
        }
        let repo = Repo {
            owner,
            name,
            version: version.to_string(),
        };
        let packages: Vec<String> = match &release["packages"] {
            Yaml::Array(a) => a
                .iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect(),
            _ => vec![repo_name.to_string()],
        };
        for p in packages {
            out.insert(p, repo.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------- archives

/// Limits for unpacking a downloaded archive.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file: u64,
    pub max_total: u64,
    pub max_entries: usize,
}

pub const LIMITS: Limits = Limits {
    max_file: 2 * 1024 * 1024,
    max_total: 96 * 1024 * 1024,
    max_entries: 500_000,
};

/// Files the extractors read; everything else in an archive is skipped.
fn wanted_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if matches!(
        name,
        "package.xml" | "CMakeLists.txt" | "setup.py" | "setup.cfg"
    ) {
        return true;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    matches!(
        ext,
        "c" | "cc"
            | "cpp"
            | "cxx"
            | "h"
            | "hh"
            | "hpp"
            | "hxx"
            | "py"
            | "xml"
            | "yaml"
            | "yml"
            | "cmake"
            | "launch"
    )
}

/// The archive path below its top-level directory, if every component is a plain name.
fn safe_rel(path: &str) -> Option<PathBuf> {
    let mut comps = path.split('/');
    comps.next()?; // `<repo>-<version>/`
    let mut out = PathBuf::new();
    let mut any = false;
    for c in comps {
        if c.is_empty()
            || c == "."
            || c == ".."
            || c.contains(['\\', ':', '\0'])
            || c.ends_with('.')
            || c.ends_with(' ')
        {
            return None;
        }
        out.push(c);
        any = true;
    }
    any.then_some(out)
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn octal(b: &[u8]) -> Option<u64> {
    if b.first().is_some_and(|x| x & 0x80 != 0) {
        return None; // base-256 sizes are only used for files far above our limits
    }
    let s = cstr(b);
    let s = s.trim_matches([' ', '\0']);
    if s.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(s, 8).ok()
}

/// Value of `key` in pax extended-header records (`"<len> <key>=<value>\n"`).
fn pax(data: &[u8], key: &str) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    text.lines().find_map(|l| {
        let (_, kv) = l.split_once(' ')?;
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool, String> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) if n == 0 => return Ok(false),
            Ok(0) => return Err("truncated archive".into()),
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(true)
}

fn skip<R: Read>(r: &mut R, n: u64) -> Result<(), String> {
    let got = std::io::copy(&mut r.take(n), &mut std::io::sink()).map_err(|e| e.to_string())?;
    if got != n {
        return Err("truncated archive".into());
    }
    Ok(())
}

/// Unpacks the wanted files of a gzipped tar archive (as served by GitHub) into `dest`.
/// Returns the number of files written and the commit recorded in the archive, if any.
pub fn extract_tar_gz<R: Read>(
    input: R,
    dest: &Path,
    lim: &Limits,
) -> Result<(usize, Option<String>), String> {
    let mut tar = flate2::read::GzDecoder::new(input);
    let mut hdr = [0u8; 512];
    let (mut files, mut total, mut entries) = (0usize, 0u64, 0usize);
    let mut next_path: Option<String> = None;
    let mut commit = None;
    loop {
        if !read_full(&mut tar, &mut hdr)? || hdr.iter().all(|&b| b == 0) {
            break;
        }
        entries += 1;
        if entries > lim.max_entries {
            return Err("archive has too many entries".into());
        }
        let size = octal(&hdr[124..136]).ok_or("invalid archive header")?;
        let padded = size.div_ceil(512) * 512;
        let typ = hdr[156];
        if matches!(typ, b'x' | b'g' | b'L') {
            if size > 1024 * 1024 {
                return Err("oversized archive metadata".into());
            }
            let mut buf = vec![0u8; padded as usize];
            if !read_full(&mut tar, &mut buf)? {
                return Err("truncated archive".into());
            }
            buf.truncate(size as usize);
            match typ {
                b'L' => next_path = Some(cstr(&buf)),
                b'x' => next_path = pax(&buf, "path").or(next_path),
                _ => commit = pax(&buf, "comment").or(commit),
            }
            continue;
        }
        let name = next_path.take().unwrap_or_else(|| {
            let n = cstr(&hdr[0..100]);
            let prefix = if &hdr[257..262] == b"ustar" {
                cstr(&hdr[345..500])
            } else {
                String::new()
            };
            if prefix.is_empty() {
                n
            } else {
                format!("{prefix}/{n}")
            }
        });
        let regular = typ == b'0' || typ == 0;
        let target = (regular && size <= lim.max_file && wanted_file(&name))
            .then(|| safe_rel(&name))
            .flatten();
        let Some(rel) = target else {
            skip(&mut tar, padded)?;
            continue;
        };
        total += size;
        if total > lim.max_total {
            return Err("archive is larger than the unpack limit".into());
        }
        let mut data = Vec::with_capacity(size as usize);
        std::io::copy(&mut (&mut tar).take(size), &mut data).map_err(|e| e.to_string())?;
        if data.len() as u64 != size {
            return Err("truncated archive".into());
        }
        skip(&mut tar, padded - size)?;
        let path = dest.join(&rel);
        // A name the file system rejects (e.g. reserved on Windows) only loses that file.
        if path
            .parent()
            .is_some_and(|p| std::fs::create_dir_all(p).is_ok())
            && std::fs::write(&path, &data).is_ok()
        {
            files += 1;
        }
    }
    Ok((files, commit))
}

// ---------------------------------------------------------------------------- planning

/// Launched packages the analysis knows nothing about: not in the workspace and not in the
/// distribution's pack (embedded or local).
pub fn missing(ix: &Index) -> Vec<String> {
    let mut v: Vec<String> = crate::workspace::referenced_packages(ix)
        .into_iter()
        .filter(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && ix.package_by_name(p).is_none()
                && !ix.pack.as_ref().is_some_and(|k| k.packages.contains_key(p))
        })
        .collect();
    v.sort();
    v
}

/// What `ensure` did, for the user.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Repositories downloaded and turned into local packs.
    pub built: Vec<(String, usize)>,
    /// Launched packages the local packs now describe.
    pub covered: Vec<String>,
    /// Launched packages not released in the distribution (or not on GitHub).
    pub unknown: Vec<String>,
    pub errors: Vec<String>,
}

const MAX_REPOS_PER_RUN: usize = 8;
const INDEX_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

fn fresh(p: &Path) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < INDEX_MAX_AGE)
}

pub fn index_url(distro: &str) -> String {
    format!("https://raw.githubusercontent.com/ros/rosdistro/master/{distro}/distribution.yaml")
}

/// Makes sure every launched package of a known distribution that rosdistro releases from
/// GitHub has a pack, downloading and building the missing ones. Packages not in the index are
/// remembered until the cached index is refreshed (weekly), so repeated runs stay offline.
pub fn ensure(ix: &Index, show_requests: bool) -> Result<Outcome, String> {
    let mut out = Outcome::default();
    let wanted = missing(ix);
    if wanted.is_empty() {
        return Ok(out);
    }
    // Packs are per distribution: parameter names change between releases.
    let Some(distro) = ix.installed.distro.clone() else {
        out.errors.push(format!(
            "no ROS distribution known (pass --distro or source ROS); not fetching packs for {}",
            wanted.join(", ")
        ));
        return Ok(out);
    };
    let dir = pack::local_dir(&distro).ok_or("no cache directory")?;
    let root = pack::cache_root().ok_or("no cache directory")?;
    let index_file = root.join("rosdistro").join(format!("{distro}.yaml"));
    let unknown_file = dir.join("not-released.txt");

    // The rosdistro index: cached for a week.
    if !fresh(&index_file) {
        let url = index_url(&distro);
        if show_requests {
            eprintln!("rosfix: online request: {url}");
        }
        let resp = crate::assist::online::run_helper(
            &crate::assist::online::FetchRequest {
                texts: vec![url.clone()],
                ..Default::default()
            },
            Duration::from_secs(60),
        )?;
        out.errors.extend(resp.errors);
        let text = resp
            .texts
            .get(&url)
            .filter(|t| t.contains("repositories:"))
            .ok_or_else(|| format!("could not download the {distro} rosdistro index"))?;
        std::fs::create_dir_all(index_file.parent().unwrap_or(&root)).map_err(|e| e.to_string())?;
        std::fs::write(&index_file, text).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&unknown_file); // a new index may release them
    }
    let yaml = std::fs::read_to_string(&index_file).map_err(|e| e.to_string())?;
    let released = parse_distribution(&yaml);
    let known_unknown: BTreeSet<String> = std::fs::read_to_string(&unknown_file)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();

    // Group the wanted packages by repository; skip repositories already built.
    let mut repos: BTreeMap<String, (Repo, Vec<String>)> = BTreeMap::new();
    for p in &wanted {
        match released.get(p) {
            Some(r) if dir.join(format!("{}.json.gz", r.key())).exists() => {}
            Some(r) => repos
                .entry(r.key())
                .or_insert_with(|| (r.clone(), Vec::new()))
                .1
                .push(p.clone()),
            None => out.unknown.push(p.clone()),
        }
    }
    if out.unknown.iter().any(|p| !known_unknown.contains(p)) {
        let all: BTreeSet<&String> = known_unknown.iter().chain(&out.unknown).collect();
        let _ = std::fs::create_dir_all(&dir);
        let body: String = all.into_iter().map(|p| format!("{p}\n")).collect();
        let _ = std::fs::write(&unknown_file, body);
    }
    out.unknown.retain(|p| !known_unknown.contains(p));
    if repos.is_empty() {
        return Ok(out);
    }

    let src_root = root.join("src").join(&distro);
    let todo: Vec<(Repo, Vec<String>)> = repos.into_values().take(MAX_REPOS_PER_RUN).collect();
    let mut req = crate::assist::online::FetchRequest::default();
    for (r, _) in &todo {
        let dest = src_root.join(r.key());
        let _ = std::fs::remove_dir_all(&dest);
        if show_requests {
            eprintln!("rosfix: online request: {}", r.tarball_url());
        }
        req.tarballs.push(crate::assist::online::TarballRequest {
            url: r.tarball_url(),
            dest,
        });
    }
    let resp = crate::assist::online::run_helper(&req, Duration::from_secs(600))?;
    out.errors.extend(resp.errors);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (r, pkgs) in todo {
        let dest = src_root.join(r.key());
        let res = resp
            .tarballs
            .get(&r.tarball_url())
            .cloned()
            .unwrap_or_default();
        if let Some(e) = res.error {
            out.errors.push(format!("{}: {e}", r.display()));
            let _ = std::fs::remove_dir_all(&dest);
            continue;
        }
        let rev = res.commit.unwrap_or_else(|| r.version.clone());
        let built = pack::build(
            &distro,
            &[(
                dest.clone(),
                format!("https://github.com/{}/{}", r.owner, r.name),
                rev,
            )],
        );
        let _ = std::fs::remove_dir_all(&dest);
        out.covered
            .extend(pkgs.into_iter().filter(|p| built.packages.contains_key(p)));
        out.built.push((r.display(), built.packages.len()));
        // Written even when it describes no node, so the repository is not downloaded again.
        std::fs::write(
            dir.join(format!("{}.json.gz", r.key())),
            pack::encode(&built),
        )
        .map_err(|e| e.to_string())?;
    }
    out.covered.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIST: &str = r#"
%YAML 1.1
---
release_platforms:
  ubuntu: [noble]
repositories:
  navigation2:
    doc: {type: git, url: https://github.com/ros-navigation/navigation2.git, version: jazzy}
    release:
      packages: [nav2_amcl, nav2_controller]
      tags: {release: 'release/jazzy/{package}/{version}'}
      url: https://github.com/ros2-gbp/navigation2-release.git
      version: 1.3.4-1
    source: {type: git, url: https://github.com/ros-navigation/navigation2.git, version: jazzy}
    status: developed
  slam_toolbox:
    release: {url: https://github.com/SteveMacenski/slam_toolbox-release.git, version: 2.8.1-1}
    source: {type: git, url: https://github.com/SteveMacenski/slam_toolbox.git, version: jazzy}
  source_only:
    source: {type: git, url: https://github.com/someone/source_only.git, version: main}
  gitlab_pkg:
    release: {url: https://gitlab.com/x/y-release.git, version: 1.0.0-1}
    source: {type: git, url: https://gitlab.com/x/y.git, version: main}
  sneaky:
    release: {url: https://github.com/a/b-release.git, version: 1.0.0-1}
    source: {type: git, url: https://github.com/a/../../evil.git, version: main}
"#;

    #[test]
    fn distribution_index() {
        let m = parse_distribution(DIST);
        let nav = &m["nav2_amcl"];
        assert_eq!(
            (nav.owner.as_str(), nav.name.as_str(), nav.version.as_str()),
            ("ros-navigation", "navigation2", "jazzy")
        );
        assert_eq!(m["nav2_controller"], *nav);
        assert_eq!(
            nav.tarball_url(),
            "https://codeload.github.com/ros-navigation/navigation2/tar.gz/jazzy"
        );
        // A released repository without a package list releases one package of its name.
        assert_eq!(m["slam_toolbox"].name, "slam_toolbox");
        // Unreleased, non-GitHub and malformed entries are left out.
        assert!(!m.contains_key("source_only"));
        assert!(!m.contains_key("gitlab_pkg"));
        assert!(!m.contains_key("sneaky"));
        for r in m.values() {
            assert!(crate::assist::online::allowed(&r.tarball_url(), None));
        }
        assert!(crate::assist::online::allowed(&index_url("jazzy"), None));
    }

    fn header(name: &str, size: usize, typ: u8) -> [u8; 512] {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        let sz = format!("{size:011o}\0");
        h[124..136].copy_from_slice(sz.as_bytes());
        h[156] = typ;
        h[257..263].copy_from_slice(b"ustar\0");
        h
    }

    fn tar_gz(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        use std::io::Write;
        let mut raw = Vec::new();
        for (name, data, typ) in entries {
            raw.extend_from_slice(&header(name, data.len(), *typ));
            raw.extend_from_slice(data);
            raw.resize(raw.len().div_ceil(512) * 512, 0);
        }
        raw.extend_from_slice(&[0u8; 1024]);
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(&raw).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn unpacks_only_safe_wanted_files() {
        let dir = tempfile::tempdir().unwrap();
        let gz = tar_gz(&[
            (
                "pax_global_header",
                b"52 comment=0123456789abcdef0123456789abcdef01234567\n",
                b'g',
            ),
            ("repo-jazzy/pkg/package.xml", b"<package/>", b'0'),
            ("repo-jazzy/pkg/src/node.cpp", b"int main(){}", b'0'),
            ("repo-jazzy/pkg/doc/logo.png", b"\x89PNG", b'0'),
            ("repo-jazzy/../escape.py", b"x", b'0'),
            ("repo-jazzy/pkg/link.py", b"", b'2'),
        ]);
        let (n, commit) = extract_tar_gz(&gz[..], dir.path(), &LIMITS).unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            commit.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert!(dir.path().join("pkg/package.xml").is_file());
        assert!(dir.path().join("pkg/src/node.cpp").is_file());
        assert!(!dir.path().join("pkg/doc/logo.png").exists());
        assert!(!dir.path().join("pkg/link.py").exists());
        assert!(!dir.path().parent().unwrap().join("escape.py").exists());
    }

    #[test]
    fn unpack_limits() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![b'a'; 4096];
        let gz = tar_gz(&[("r/a.py", &big[..], b'0'), ("r/b.py", &big[..], b'0')]);
        // A file above the per-file limit is skipped; the total limit fails the archive.
        let small = Limits {
            max_file: 1000,
            ..LIMITS
        };
        assert_eq!(extract_tar_gz(&gz[..], dir.path(), &small).unwrap().0, 0);
        let tight = Limits {
            max_total: 5000,
            ..LIMITS
        };
        assert!(extract_tar_gz(&gz[..], dir.path(), &tight).is_err());
        // A truncated archive is an error, not a partial success.
        let raw_trunc = &gz[..gz.len() / 2];
        assert!(extract_tar_gz(raw_trunc, dir.path(), &LIMITS).is_err());
    }

    #[test]
    fn path_checks() {
        assert_eq!(
            safe_rel("top/a/b.cpp"),
            Some(PathBuf::from("a").join("b.cpp"))
        );
        for bad in [
            "top", "top/../x", "top/a//b", "top/C:/x", "top/a\\b", "top/x.", "top/./x",
        ] {
            assert_eq!(safe_rel(bad), None, "{bad}");
        }
        assert!(wanted_file("x/CMakeLists.txt") && wanted_file("x/n.hpp"));
        assert!(!wanted_file("x/README.md") && !wanted_file("x/mesh.stl"));
    }
}
