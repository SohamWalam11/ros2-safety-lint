//! Workspace discovery and indexing: find packages and relevant files, read them
//! once, run the per-file extractors, and assemble package facts.

use crate::config::Config;
use crate::extract::{self, launch, params, urdf};
use crate::facts::*;
use crate::installed::{self, InstalledEnv};
use crate::source::{normalize_path, FileId, Loc, SourceFile, SourceMap};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "build",
    "install",
    "log",
    "target",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".cache",
    ".rosfix",
];
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    Manifest,
    CMakeLists,
    SetupPy,
    SetupCfg,
    LaunchPy,
    LaunchXml,
    LaunchYaml,
    ParamYaml,
    Rviz,
    Urdf,
    Cpp,
    Python,
    Security,
    /// BehaviorTree.CPP XML (Nav2 behavior trees).
    BtXml,
    /// MoveIt configuration: SRDF and kinematics / joint_limits / moveit_controllers YAML.
    MoveIt,
    /// ros_gz_bridge YAML configuration (a sequence of bridged topics).
    GzBridge,
    /// Python or C++ that uses only ROS 1 client libraries (`rospy`, `ros/ros.h`).
    Ros1,
    /// Loaded on demand (plugin XML, param-library specs).
    Other,
}

#[derive(Debug, Default, Clone)]
pub struct IndexStats {
    pub files_seen: usize,
    pub files_loaded: usize,
    pub files_parsed: usize,
    /// Files whose facts came from the extraction cache instead of being parsed again.
    pub files_reused: usize,
    pub bytes_loaded: usize,
}

#[derive(Debug, Default)]
pub struct Index {
    pub root: PathBuf,
    pub sources: SourceMap,
    pub kinds: Vec<FileKind>,
    pub packages: Vec<Package>,
    /// file id → index into packages
    pub file_pkg: Vec<Option<usize>>,
    pub code: std::collections::BTreeMap<FileId, CodeFacts>,
    pub launches: std::collections::BTreeMap<FileId, LaunchFile>,
    pub param_files: std::collections::BTreeMap<FileId, ParamFile>,
    pub robots: Vec<RobotModel>,
    pub security: SecurityFacts,
    pub rviz: std::collections::BTreeMap<FileId, Vec<RvizSubscription>>,
    /// Behavior-tree XML files.
    pub bt_files: std::collections::BTreeMap<FileId, BtFile>,
    /// generate_parameter_library specs: workspace-relative path → (decls, open)
    pub param_specs: std::collections::BTreeMap<String, (Vec<ParamDecl>, bool)>,
    pub parse_errors: Vec<(FileId, String)>,
    /// Enclave paths found as `<keystore>/enclaves/<path>/` directories.
    pub keystore_enclaves: Vec<String>,
    /// Workspace-relative paths of `.msg`/`.srv`/`.action` files (not parsed).
    pub interface_files: Vec<String>,
    pub stats: IndexStats,
    /// The user's installed ROS 2 environment (empty if none was found).
    pub installed: InstalledEnv,
    /// Files loaded from installed packages: used for resolution, never reported on.
    pub external_files: HashSet<FileId>,
    /// Knowledge pack for the detected distribution, if any.
    pub pack: Option<crate::pack::Pack>,
    /// (package, executable or component class) → synthetic node impls from the pack,
    /// with (params_open, params_open_dotted).
    pub pack_links: std::collections::BTreeMap<(String, String), PackLink>,
}

/// Synthetic node impls from a knowledge pack, with (params_open, params_open_dotted).
pub type PackLink = (Vec<(FileId, usize)>, bool, bool);

/// Inputs to indexing besides the workspace root.
#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    pub jobs: usize,
    /// In-memory file contents that replace what is on disk (fix verification).
    pub overrides: HashMap<PathBuf, String>,
    /// Installed ROS prefixes (normally from `AMENT_PREFIX_PATH`).
    pub prefixes: Vec<PathBuf>,
    pub distro: Option<String>,
    /// Facts extracted by earlier runs, reused for files whose content did not change.
    pub cache: Option<std::sync::Arc<ExtractCache>>,
}

/// Memo of per-file extraction results, keyed by file id and a hash of path + content.
///
/// Extraction is a pure function of (id, path, text) for every kind except URDF/xacro,
/// which can include other files and is therefore never cached. Re-analysing a workspace
/// after an edit (fix verification, the mutation benchmark) re-parses only edited files.
#[derive(Default)]
pub struct ExtractCache {
    map: std::sync::Mutex<HashMap<(FileId, u64), Extracted>>,
}

impl std::fmt::Debug for ExtractCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.map.lock().map(|m| m.len()).unwrap_or(0);
        write!(f, "ExtractCache({n} files)")
    }
}

/// MoveIt configuration files that carry no `ros__parameters` level.
const MOVEIT_YAML: &[&str] = &[
    "kinematics.yaml",
    "joint_limits.yaml",
    "moveit_controllers.yaml",
];

fn classify(rel: &str) -> Option<FileKind> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "package.xml" => FileKind::Manifest,
        "cmakelists.txt" => FileKind::CMakeLists,
        "setup.py" => FileKind::SetupPy,
        "setup.cfg" => FileKind::SetupCfg,
        n if n.ends_with(".srdf") || n.ends_with(".srdf.xacro") => FileKind::MoveIt,
        n if MOVEIT_YAML.contains(&n) => FileKind::MoveIt,
        _ => {
            let ext = lower.rsplit('.').next().unwrap_or("");
            match ext {
                "py" => FileKind::Python,                // may be refined to LaunchPy
                "xml" | "launch" => FileKind::LaunchXml, // refined by content
                "yaml" | "yml" => FileKind::ParamYaml,   // refined by content
                "rviz" => FileKind::Rviz,
                "urdf" | "xacro" => FileKind::Urdf,
                "cpp" | "cc" | "cxx" | "hpp" | "h" | "hh" | "hxx" => FileKind::Cpp,
                _ => return None,
            }
        }
    })
}

/// Refines the kind using content; returns None if the file is irrelevant.
fn refine(kind: FileKind, rel: &str, text: &str) -> Option<FileKind> {
    match kind {
        FileKind::Python => {
            if launch::is_python_launch(rel, text) {
                Some(FileKind::LaunchPy)
            } else if extract::python::is_relevant(text) {
                Some(FileKind::Python)
            } else if crate::agents::ros1::ros1_python_import(text).is_some() {
                Some(FileKind::Ros1)
            } else {
                None
            }
        }
        FileKind::LaunchXml => {
            if launch::is_xml_launch(rel, text) {
                Some(FileKind::LaunchXml)
            } else if extract::sros2::is_relevant(text) {
                Some(FileKind::Security)
            } else if extract::bt::is_relevant(text) {
                Some(FileKind::BtXml)
            } else {
                None
            }
        }
        FileKind::ParamYaml => {
            if launch::is_yaml_launch(rel, text) {
                Some(FileKind::LaunchYaml)
            } else if crate::agents::gzbridge::is_bridge_config(text) {
                Some(FileKind::GzBridge)
            } else if params::looks_like_param_file(text) {
                Some(FileKind::ParamYaml)
            } else {
                None
            }
        }
        FileKind::Cpp if extract::cpp::is_relevant(text) => Some(FileKind::Cpp),
        FileKind::Cpp => crate::agents::ros1::ros1_cpp_include(text).map(|_| FileKind::Ros1),
        // A ros2_control joint_limits.yaml with `ros__parameters` is an ordinary parameter file.
        FileKind::MoveIt
            if !rel.ends_with("srdf")
                && !rel.ends_with(".xacro")
                && params::looks_like_param_file(text) =>
        {
            Some(FileKind::ParamYaml)
        }
        other => Some(other),
    }
}

fn rel_of(root: &Path, p: &Path) -> String {
    let r = p.strip_prefix(root).unwrap_or(p);
    r.to_string_lossy().replace('\\', "/")
}

impl Index {
    pub fn file(&self, id: FileId) -> &SourceFile {
        self.sources.get(id)
    }

    /// Hash of a file's path and content (cache key and change detection).
    pub fn content_hash(&self, id: FileId) -> u64 {
        let f = self.sources.get(id);
        crate::util::fnv1a(f.rel_path.as_bytes())
            ^ crate::util::fnv1a(f.text.as_bytes()).rotate_left(1)
    }

    pub fn package_by_name(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn package_of(&self, id: FileId) -> Option<&Package> {
        self.file_pkg
            .get(id)
            .copied()
            .flatten()
            .map(|i| &self.packages[i])
    }

    /// Finds a loaded file by workspace-relative path.
    pub fn find_rel(&self, rel: &str) -> Option<FileId> {
        let n = normalize_path(Path::new(rel));
        self.sources
            .files
            .iter()
            .position(|f| normalize_path(Path::new(&f.rel_path)) == n)
    }

    /// Lists loaded files under a workspace-relative directory.
    pub fn files_under<'s>(&'s self, dir: &'s str) -> impl Iterator<Item = FileId> + 's {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        self.sources
            .files
            .iter()
            .enumerate()
            .filter(move |(_, f)| f.rel_path.starts_with(&prefix))
            .map(|(i, _)| i)
    }

    /// True if a workspace-relative path exists on disk (loaded or not).
    pub fn exists_rel(&self, rel: &str) -> bool {
        self.root.join(rel).exists()
    }
}

/// Discovers and indexes a workspace rooted at `root` (or a single file).
pub fn index(root: &Path, config: &Config, opts: &IndexOptions) -> Index {
    let jobs = opts.jobs.max(1);
    let overrides = &opts.overrides;
    let root = if root.is_file() {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        root.to_path_buf()
    };
    let mut ix = Index {
        root: root.clone(),
        ..Default::default()
    };

    // ----------------------------------------------------------------- discovery
    let walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            if e.file_type().is_dir() {
                if SKIP_DIRS.contains(&name.as_ref()) {
                    return false;
                }
                // colcon semantics: a marker file excludes the directory.
                if e.path().join("COLCON_IGNORE").exists()
                    || e.path().join("AMENT_IGNORE").exists()
                    || e.path().join("CATKIN_IGNORE").exists()
                {
                    return false;
                }
            }
            true
        });
    let mut keystore_dirs = Vec::new();
    for entry in walker.filter_map(Result::ok) {
        let path = entry.path();
        let rel = rel_of(&root, path);
        if config.ignore.is_match(&rel) {
            continue;
        }
        if entry.file_type().is_dir() {
            // keystore/enclaves/<path>/permissions.xml or cert.pem → an enclave
            if path.join("permissions.p7s").exists() || path.join("cert.pem").exists() {
                let padded = format!("/{rel}");
                if let Some(i) = padded.find("/enclaves/") {
                    keystore_dirs.push(padded[i + "/enclaves".len()..].to_string());
                }
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        ix.stats.files_seen += 1;
        if [".msg", ".srv", ".action"].iter().any(|e| rel.ends_with(e)) {
            ix.interface_files.push(rel.clone());
        }
        let Some(kind) = classify(&rel) else { continue };
        if entry
            .metadata()
            .map(|m| m.len() > MAX_FILE_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        let text = match overrides.get(path) {
            Some(t) => t.clone(),
            None => match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(_) => continue,
            },
        };
        let Some(kind) = refine(kind, &rel, &text) else {
            continue;
        };
        ix.stats.files_loaded += 1;
        ix.stats.bytes_loaded += text.len();
        ix.sources
            .add(SourceFile::new(rel, path.to_path_buf(), text));
        ix.kinds.push(kind);
    }
    ix.keystore_enclaves = keystore_dirs;

    // ----------------------------------------------------------------- packages
    let manifests: Vec<FileId> = (0..ix.sources.files.len())
        .filter(|i| ix.kinds[*i] == FileKind::Manifest)
        .collect();
    let mut pkg_dirs: Vec<(String, FileId)> = manifests
        .iter()
        .map(|&id| {
            let rel = &ix.sources.get(id).rel_path;
            (
                rel.rsplit_once('/')
                    .map_or(String::new(), |(d, _)| d.to_string()),
                id,
            )
        })
        .collect();
    pkg_dirs.sort_by_key(|(d, _)| std::cmp::Reverse(d.len()));
    let owner = |rel: &str| -> Option<FileId> {
        pkg_dirs
            .iter()
            .find(|(d, _)| d.is_empty() || rel.starts_with(&format!("{d}/")))
            .map(|(_, m)| *m)
    };
    let mut manifest_to_pkg: HashMap<FileId, usize> = HashMap::new();
    for &m in &manifests {
        let text = ix.sources.get(m).text.clone();
        match extract::package::parse_manifest(m, &text) {
            Ok(man) => {
                let rel = ix.sources.get(m).rel_path.clone();
                let dir = rel
                    .rsplit_once('/')
                    .map_or(String::new(), |(d, _)| d.to_string());
                let build_type = man.build_type.unwrap_or(BuildType::AmentCmake);
                manifest_to_pkg.insert(m, ix.packages.len());
                ix.packages.push(Package {
                    name: man.name,
                    dir,
                    manifest: m,
                    manifest_name_loc: Loc::new(m, man.name_loc.0, man.name_loc.1),
                    build_type,
                    deps: man.deps,
                    executables: Vec::new(),
                    executables_complete: true,
                    installed_share_dirs: Vec::new(),
                    install_rules_complete: true,
                    uses: Vec::new(),
                    plugin_xmls: man.plugin_xmls,
                    ros1_support: man.ros1_support,
                    plugin_classes: Vec::new(),
                    param_lib_specs: Vec::new(),
                    python: None,
                    cmake_file: None,
                    rosidl: None,
                    uninstalled_component_libs: Vec::new(),
                    install_sources: Vec::new(),
                    setup_py_packages: None,
                });
            }
            Err(e) => ix.parse_errors.push((m, e)),
        }
    }
    ix.file_pkg = (0..ix.sources.files.len())
        .map(|i| owner(&ix.sources.get(i).rel_path).and_then(|m| manifest_to_pkg.get(&m).copied()))
        .collect();

    // Build files per package.
    for i in 0..ix.sources.files.len() {
        let Some(p) = ix.file_pkg[i] else { continue };
        let f = ix.sources.get(i);
        let file_dir = f.rel_path.rsplit_once('/').map_or("", |(d, _)| d);
        let in_pkg_root = file_dir == ix.packages[p].dir;
        if !in_pkg_root {
            // A CMakeLists.txt in a subdirectory (`add_subdirectory`, as in moveit_core) can
            // still generate parameter libraries; take only those, since its targets may be
            // conditional.
            if ix.kinds[i] == FileKind::CMakeLists {
                let name = ix.packages[p].name.clone();
                let cf = extract::cmake::interpret(i, &f.text, &name, file_dir);
                ix.packages[p].param_lib_specs.extend(cf.param_specs);
            }
            continue;
        }
        let (name, dir) = (ix.packages[p].name.clone(), ix.packages[p].dir.clone());
        match ix.kinds[i] {
            FileKind::CMakeLists => {
                let cf = extract::cmake::interpret(i, &f.text, &name, &dir);
                let pk = &mut ix.packages[p];
                pk.cmake_file = Some(i);
                pk.executables.extend(cf.executables);
                pk.executables_complete &= cf.complete;
                pk.installed_share_dirs.extend(cf.share_dirs);
                pk.install_rules_complete &= cf.install_complete;
                pk.uses.extend(cf.uses);
                pk.param_lib_specs.extend(cf.param_specs);
                pk.plugin_classes.extend(cf.components);
                if cf.rosidl.is_some() {
                    pk.rosidl = cf.rosidl;
                }
                pk.uninstalled_component_libs
                    .extend(cf.uninstalled_component_libs);
                pk.install_sources.extend(cf.install_sources);
                for x in cf.plugin_xmls {
                    if !pk.plugin_xmls.contains(&x) {
                        pk.plugin_xmls.push(x);
                    }
                }
            }
            FileKind::SetupPy => match extract::package::parse_setup_py(i, &f.text, &name, &dir) {
                Ok(sp) => {
                    let cfg = ix.sources.files.iter().enumerate().find(|(j, g)| {
                        ix.kinds[*j] == FileKind::SetupCfg
                            && ix.file_pkg[*j] == Some(p)
                            && g.rel_path.rsplit_once('/').map_or("", |(d, _)| d) == dir
                    });
                    let info = extract::package::python_info(
                        Some(i),
                        cfg.map(|(j, g)| (j, g.text.as_str())),
                        Some(&sp),
                    );
                    let pk = &mut ix.packages[p];
                    pk.executables.extend(sp.executables);
                    pk.installed_share_dirs.extend(sp.share_dirs);
                    pk.install_rules_complete &= sp.data_files_complete;
                    pk.setup_py_packages = sp.packages;
                    pk.python = Some(info);
                }
                Err(e) => {
                    ix.parse_errors.push((i, e));
                    ix.packages[p].executables_complete = false;
                    ix.packages[p].install_rules_complete = false;
                }
            },
            _ => {}
        }
    }
    // Plugin XMLs and parameter-library specs (loaded from disk on demand).
    for p in 0..ix.packages.len() {
        let dir = ix.packages[p].dir.clone();
        let join = |r: &str| {
            if dir.is_empty() {
                r.to_string()
            } else {
                format!("{dir}/{r}")
            }
        };
        let mut classes = Vec::new();
        for x in ix.packages[p].plugin_xmls.clone() {
            if let Ok(t) = std::fs::read_to_string(root.join(join(&x))) {
                classes.extend(extract::package::parse_plugin_xml(&t));
            }
        }
        ix.packages[p].plugin_classes.extend(classes);
        for spec in ix.packages[p].param_lib_specs.clone() {
            let full = root.join(&spec);
            if let Ok(t) = std::fs::read_to_string(&full) {
                let fid = match ix.find_rel(&spec) {
                    Some(id) => id,
                    None => {
                        let id =
                            ix.sources
                                .add(SourceFile::new(spec.clone(), full.clone(), t.clone()));
                        ix.kinds.push(FileKind::Other);
                        ix.file_pkg.push(Some(p));
                        id
                    }
                };
                let parsed = extract::package::parse_param_lib_spec(fid, &t);
                ix.param_specs.insert(spec, parsed);
            }
        }
    }

    // ----------------------------------------------------------------- per-file extraction
    let ids: Vec<FileId> = (0..ix.sources.files.len()).collect();
    absorb(&mut ix, ids, jobs, opts.cache.as_deref());

    // ----------------------------------------------------------------- installed ROS
    if !opts.prefixes.is_empty() {
        ix.installed = installed::discover(&opts.prefixes, opts.distro.clone());
        load_installed_launch_files(&mut ix, jobs);
    } else if opts.distro.is_some() {
        ix.installed.distro = opts.distro.clone();
    }
    if let Some(d) = ix.installed.distro.clone() {
        ix.pack = crate::pack::embedded(&d);
    }
    // NoDL documents that installed packages register describe executables the pack does not.
    if !ix.installed.packages.is_empty() {
        let wanted = referenced_packages(&ix);
        crate::nodl::merge_installed(&mut ix, &wanted);
    }
    if ix.pack.is_some() {
        inject_pack(&mut ix);
    }

    // Python imports and C++ includes → package uses.
    let mut uses: Vec<(usize, PackageUse)> = Vec::new();
    for (id, c) in &ix.code {
        let Some(p) = ix.file_pkg[*id] else { continue };
        let rel = &ix.sources.get(*id).rel_path;
        if config.is_test_path(rel) {
            continue;
        }
        if ix.kinds[*id] == FileKind::Python {
            for (m, loc) in &c.imports {
                uses.push((
                    p,
                    PackageUse {
                        name: m.clone(),
                        kind: UseKind::PythonImport,
                        loc: *loc,
                    },
                ));
            }
        }
    }
    // Imports in Python launch files are runtime dependencies too (`launch_ros`, `nav2_common`,
    // …). `launch` and `ament_index_python` come with `launch_ros`, so they are not required.
    static IMPORT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let import = IMPORT.get_or_init(|| {
        regex::Regex::new(r"(?m)^[ 	]*(?:from[ 	]+([A-Za-z_]\w*)|import[ 	]+([A-Za-z_]\w*))")
            .expect("valid regex")
    });
    for id in ix.launches.keys() {
        let (Some(p), FileKind::LaunchPy) = (ix.file_pkg[*id], ix.kinds[*id]) else {
            continue;
        };
        let f = ix.sources.get(*id);
        if config.is_test_path(&f.rel_path) {
            continue;
        }
        for c in import.captures_iter(&f.text) {
            let Some(m) = c.get(1).or_else(|| c.get(2)) else {
                continue;
            };
            if matches!(m.as_str(), "launch" | "ament_index_python") {
                continue;
            }
            uses.push((
                p,
                PackageUse {
                    name: m.as_str().to_string(),
                    kind: UseKind::LaunchImport,
                    loc: Loc::new(*id, m.start(), m.end()),
                },
            ));
        }
    }
    for (p, u) in uses {
        ix.packages[p].uses.push(u);
    }
    ix
}

/// Packages referenced by launch files (node packages and `$(find-pkg-share …)` paths).
fn referenced_packages(ix: &Index) -> HashSet<String> {
    fn walk_lval(v: &LVal, out: &mut HashSet<String>) {
        match v {
            LVal::PkgShare(p) | LVal::PkgPrefix(p) => {
                out.insert(p.clone());
            }
            LVal::Concat(xs) | LVal::PathJoin(xs) => xs.iter().for_each(|x| walk_lval(x, out)),
            _ => {}
        }
    }
    fn walk(items: &[LaunchItem], out: &mut HashSet<String>) {
        for it in items {
            match it {
                LaunchItem::Node(n) => {
                    if let LVal::Lit(p) = &n.pkg {
                        out.insert(p.clone());
                    }
                    for s in &n.params {
                        if let LaunchParamSource::File { path, .. } = s {
                            walk_lval(path, out);
                        }
                    }
                    n.arguments.iter().for_each(|a| walk_lval(a, out));
                }
                LaunchItem::Include { path, .. } => walk_lval(path, out),
                LaunchItem::Group { items, .. } => walk(items, out),
                _ => {}
            }
        }
    }
    let mut out = HashSet::new();
    for lf in ix.launches.values() {
        walk(&lf.items, &mut out);
    }
    out
}

/// Loads launch, parameter and rviz files of installed packages that the workspace refers
/// to (transitively, a few levels deep), so includes and parameter files resolve.
fn load_installed_launch_files(ix: &mut Index, jobs: usize) {
    const MAX_FILES_PER_PKG: usize = 300;
    let mut loaded_pkgs: HashSet<String> = HashSet::new();
    for _round in 0..4 {
        let wanted: Vec<String> = referenced_packages(ix)
            .into_iter()
            .filter(|p| {
                ix.package_by_name(p).is_none()
                    && ix.installed.get(p).is_some()
                    && !loaded_pkgs.contains(p)
            })
            .collect();
        if wanted.is_empty() {
            break;
        }
        let mut new_ids = Vec::new();
        for pkg in wanted {
            loaded_pkgs.insert(pkg.clone());
            let share = ix
                .installed
                .get(&pkg)
                .map(|p| p.share.clone())
                .unwrap_or_default();
            let mut n = 0;
            for e in walkdir::WalkDir::new(&share)
                .max_depth(4)
                .into_iter()
                .filter_map(Result::ok)
            {
                if n >= MAX_FILES_PER_PKG || !e.file_type().is_file() {
                    continue;
                }
                let sub = e
                    .path()
                    .strip_prefix(&share)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                let rel = format!("<ros>/{pkg}/{sub}");
                let Some(kind) = classify(&rel) else { continue };
                if matches!(
                    kind,
                    FileKind::Cpp | FileKind::Manifest | FileKind::CMakeLists
                ) {
                    continue;
                }
                if e.metadata()
                    .map(|m| m.len() > MAX_FILE_BYTES)
                    .unwrap_or(true)
                {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(e.path()) else {
                    continue;
                };
                // Only launch/param/rviz/description files are kept (not Python modules).
                let Some(kind) = refine(kind, &format!("{pkg}/{sub}"), &text) else {
                    continue;
                };
                if !matches!(
                    kind,
                    FileKind::LaunchPy
                        | FileKind::LaunchXml
                        | FileKind::LaunchYaml
                        | FileKind::ParamYaml
                        | FileKind::Rviz
                        | FileKind::Urdf
                ) {
                    continue;
                }
                let id = ix
                    .sources
                    .add(SourceFile::new(rel, e.path().to_path_buf(), text));
                ix.kinds.push(kind);
                ix.file_pkg.push(None);
                ix.external_files.insert(id);
                new_ids.push(id);
                n += 1;
            }
        }
        absorb(ix, new_ids, jobs, None);
    }
}

/// Adds synthetic node implementations from the knowledge pack for launched packages that
/// are not in the workspace, so parameter/QoS checks also cover installed nodes.
fn inject_pack(ix: &mut Index) {
    let Some(pack) = ix.pack.take() else { return };
    let wanted: Vec<String> = referenced_packages(ix)
        .into_iter()
        .filter(|p| ix.package_by_name(p).is_none() && pack.packages.contains_key(p))
        .collect();
    for pkg in wanted {
        let pp = &pack.packages[&pkg];
        let rel = format!("<rosfix knowledge pack: {}>/{pkg}", pack.distro);
        let fid = ix
            .sources
            .add(SourceFile::new(rel, PathBuf::new(), String::new()));
        ix.kinds.push(FileKind::Other);
        ix.file_pkg.push(None);
        ix.external_files.insert(fid);
        let loc = Loc::new(fid, 0, 0);
        let mut facts = CodeFacts::default();
        for (key, n) in pp.executables.iter().chain(pp.components.iter()) {
            let imp = NodeImpl {
                file: fid,
                lang: Lang::Cpp,
                class_name: Some(key.clone()),
                base: None,
                default_name: n.default_name.clone(),
                params: n
                    .params
                    .iter()
                    .map(|p| ParamDecl {
                        name: p.name.clone(),
                        ty: p.ty,
                        has_default: p.has_default,
                        default_text: None,
                        loc,
                        constraints: Vec::new(),
                    })
                    .collect(),
                params_open: n.params_open,
                endpoints: n
                    .endpoints
                    .iter()
                    .map(|e| Endpoint {
                        kind: e.kind,
                        name: Some(e.name.clone()),
                        msg_type: e.msg_type.clone(),
                        qos: e.qos.clone(),
                        qos_overrides_enabled: e.qos_overrides_enabled,
                        var: None,
                        group: None,
                        callback: None,
                        loc,
                    })
                    .collect(),
                callbacks: Vec::new(),
                groups: Vec::new(),
                dedicated_groups: Vec::new(),
                loc,
            };
            facts.nodes.push(imp);
            ix.pack_links.insert(
                (pkg.clone(), key.clone()),
                (
                    vec![(fid, facts.nodes.len() - 1)],
                    n.params_open,
                    n.params_open_dotted,
                ),
            );
        }
        ix.code.insert(fid, facts);
    }
    ix.pack = Some(pack);
}

/// Runs the extractors on `ids` and merges the results into the index.
fn absorb(ix: &mut Index, ids: Vec<FileId>, jobs: usize, cache: Option<&ExtractCache>) {
    // Cache lookups first; only misses are parsed.
    let key = |ix: &Index, id: FileId| (id, ix.content_hash(id));
    let cacheable = |ix: &Index, id: FileId| ix.kinds[id] != FileKind::Urdf;
    let mut results: Vec<Option<Extracted>> = vec![None; ids.len()];
    if let Some(c) = cache {
        let map = c.map.lock().expect("extract cache");
        for (slot, &id) in results.iter_mut().zip(&ids) {
            if cacheable(ix, id) {
                *slot = map.get(&key(ix, id)).cloned();
            }
        }
    }
    let misses: Vec<FileId> = ids
        .iter()
        .zip(&results)
        .filter(|(_, r)| r.is_none())
        .map(|(id, _)| *id)
        .collect();
    ix.stats.files_reused += ids.len() - misses.len();
    let mut fresh = run_extractors(ix, &misses, jobs).into_iter();
    if let Some(c) = cache {
        let mut map = c.map.lock().expect("extract cache");
        let fresh_vec: Vec<Extracted> = fresh.collect();
        for (&id, r) in misses.iter().zip(&fresh_vec) {
            if cacheable(ix, id) {
                map.insert(key(ix, id), r.clone());
            }
        }
        fresh = fresh_vec.into_iter();
    }
    // (result, came from the cache)
    let results: Vec<(Extracted, bool)> = results
        .into_iter()
        .map(|r| match r {
            Some(r) => (r, true),
            None => (fresh.next().expect("one result per miss"), false),
        })
        .collect();
    for (id, (r, cached)) in ids.into_iter().zip(results) {
        match r {
            Extracted::None => {}
            Extracted::Err(e) => ix.parse_errors.push((id, e)),
            Extracted::Code(c) => {
                ix.stats.files_parsed += usize::from(!cached);
                ix.code.insert(id, c);
            }
            Extracted::Launch(l) => {
                ix.stats.files_parsed += usize::from(!cached);
                ix.launches.insert(id, l);
            }
            Extracted::Params(p) => {
                ix.stats.files_parsed += usize::from(!cached);
                ix.param_files.insert(id, p);
            }
            Extracted::Robot(r) => {
                ix.stats.files_parsed += usize::from(!cached);
                if let Some(r) = r {
                    ix.robots.push(r);
                }
            }
            Extracted::Security(s) => {
                ix.stats.files_parsed += usize::from(!cached);
                ix.security.governance_weak.extend(s.governance_weak);
                ix.security.grants.extend(s.grants);
                ix.security.policy_profiles.extend(s.policy_profiles);
                for e in s.enclaves {
                    if !ix.security.enclaves.contains(&e) {
                        ix.security.enclaves.push(e);
                    }
                }
            }
            Extracted::Bt(b) => {
                ix.stats.files_parsed += usize::from(!cached);
                ix.bt_files.insert(id, b);
            }
            Extracted::Rviz(v) => {
                ix.stats.files_parsed += usize::from(!cached);
                ix.rviz.insert(id, v);
            }
        }
    }
}

#[derive(Clone)]
enum Extracted {
    None,
    Err(String),
    Code(CodeFacts),
    Launch(LaunchFile),
    Params(ParamFile),
    Robot(Option<RobotModel>),
    Security(SecurityFacts),
    Rviz(Vec<RvizSubscription>),
    Bt(BtFile),
}

struct WsLoader<'a> {
    ix: &'a Index,
}

impl<'a> urdf::Loader<'a> for WsLoader<'a> {
    fn load(&self, from: FileId, path: &str) -> Option<(FileId, &'a str)> {
        let rel = if let Some(rest) = path.strip_prefix("@pkg:") {
            let (pkg, sub) = rest.split_once('/').unwrap_or((rest, ""));
            let p = self.ix.package_by_name(pkg)?;
            if p.dir.is_empty() {
                sub.to_string()
            } else {
                format!("{}/{sub}", p.dir)
            }
        } else if let Some(rest) = path.strip_prefix("package://") {
            let (pkg, sub) = rest.split_once('/').unwrap_or((rest, ""));
            let p = self.ix.package_by_name(pkg)?;
            if p.dir.is_empty() {
                sub.to_string()
            } else {
                format!("{}/{sub}", p.dir)
            }
        } else {
            let from_rel = &self.ix.sources.get(from).rel_path;
            let dir = from_rel.rsplit_once('/').map_or("", |(d, _)| d);
            if dir.is_empty() {
                path.to_string()
            } else {
                format!("{dir}/{path}")
            }
        };
        let id = self.ix.find_rel(&rel)?;
        Some((id, self.ix.sources.get(id).text.as_str()))
    }
}

fn extract_one(ix: &Index, id: FileId) -> Extracted {
    let f = ix.sources.get(id);
    let res = match ix.kinds[id] {
        FileKind::Cpp => extract::cpp::extract(id, &f.text).map(Extracted::Code),
        FileKind::Python => extract::python::extract(id, &f.rel_path, &f.text).map(Extracted::Code),
        FileKind::LaunchPy => launch::parse_python(id, &f.rel_path, &f.text).map(Extracted::Launch),
        FileKind::LaunchXml => launch::parse_xml(id, &f.rel_path, &f.text).map(Extracted::Launch),
        FileKind::LaunchYaml => launch::parse_yaml(id, &f.rel_path, &f.text).map(Extracted::Launch),
        FileKind::ParamYaml => {
            extract::yaml::parse(&f.text).map(|d| Extracted::Params(params::extract(id, &d)))
        }
        FileKind::Urdf => urdf::extract(id, &f.text, &WsLoader { ix }).map(Extracted::Robot),
        FileKind::Rviz => extract::rviz::extract(id, &f.text).map(Extracted::Rviz),
        FileKind::BtXml => extract::bt::extract(id, &f.text).map(Extracted::Bt),
        FileKind::Security => {
            let mut s = SecurityFacts::default();
            extract::sros2::extract(id, &f.text, &mut s).map(|_| Extracted::Security(s))
        }
        _ => Ok(Extracted::None),
    };
    res.unwrap_or_else(Extracted::Err)
}

#[cfg(feature = "parallel")]
fn run_extractors(ix: &Index, ids: &[FileId], jobs: usize) -> Vec<Extracted> {
    use rayon::prelude::*;
    if jobs > 1 {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(jobs).build();
        if let Ok(pool) = pool {
            return pool.install(|| ids.par_iter().map(|&id| extract_one(ix, id)).collect());
        }
    }
    ids.iter().map(|&id| extract_one(ix, id)).collect()
}

#[cfg(not(feature = "parallel"))]
fn run_extractors(ix: &Index, ids: &[FileId], _jobs: usize) -> Vec<Extracted> {
    ids.iter().map(|&id| extract_one(ix, id)).collect()
}
