//! The resolver: turns launch facts into a launch graph of node instances with fully
//! qualified names, bound parameters, remaps, and links to their implementation.

use crate::extract::launch::WS_PREFIX;
use crate::facts::*;
use crate::source::{FileId, Loc};
use crate::workspace::Index;
use std::collections::{HashMap, HashSet};

// ============================================================================ names

/// Joins namespace pieces following ROS 2 rules (absolute pieces reset).
pub fn join_ns(base: &str, piece: &str) -> String {
    let piece = piece.trim();
    if piece.is_empty() {
        return normalize_ns(base);
    }
    if piece.starts_with('/') {
        return normalize_ns(piece);
    }
    let b = normalize_ns(base);
    if b == "/" {
        normalize_ns(&format!("/{piece}"))
    } else {
        normalize_ns(&format!("{b}/{piece}"))
    }
}

pub fn normalize_ns(ns: &str) -> String {
    let parts: Vec<&str> = ns.split('/').filter(|p| !p.is_empty()).collect();
    format!("/{}", parts.join("/"))
}

/// Expands a topic/service name relative to a node (`~` = private namespace).
pub fn expand_name(name: &str, ns: &str, node: &str) -> String {
    if name.starts_with('/') {
        return normalize_ns(name);
    }
    if let Some(rest) = name.strip_prefix('~') {
        let fq = join_ns(ns, node);
        return normalize_ns(&format!("{fq}/{}", rest.trim_start_matches('/')));
    }
    join_ns(ns, name)
}

/// rcl-style matching of a parameter-file node key against a fully qualified node name.
/// Keys without a leading slash are treated as absolute. Supports `*` (one token) and `**` (any).
pub fn key_matches(key: &str, fq_name: &str) -> bool {
    let key = if key.starts_with('/') {
        key.to_string()
    } else {
        format!("/{key}")
    };
    let kt: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
    let nt: Vec<&str> = fq_name.split('/').filter(|s| !s.is_empty()).collect();
    glob_tokens(&kt, &nt)
}

fn glob_tokens(k: &[&str], n: &[&str]) -> bool {
    match (k.first(), n.first()) {
        (None, None) => true,
        (Some(&"**"), _) => glob_tokens(&k[1..], n) || (!n.is_empty() && glob_tokens(k, &n[1..])),
        (Some(t), Some(m)) => tok_match(t, m) && glob_tokens(&k[1..], &n[1..]),
        _ => false,
    }
}

fn tok_match(pattern: &str, s: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == s;
    }
    // simple glob with '*' inside a token
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0;
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        match s[pos..].find(p) {
            Some(f) if i > 0 || f == 0 => pos += f + p.len(),
            _ => return false,
        }
    }
    parts.last().is_none_or(|l| l.is_empty() || s.ends_with(l))
}

// ============================================================================ graph types

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplStatus {
    /// Linked to source code in the workspace.
    Resolved,
    /// Package not in the workspace (installed elsewhere).
    External,
    /// Package in the workspace, but the executable/class could not be linked.
    Unresolved,
}

#[derive(Debug, Clone)]
pub struct BoundParam {
    pub name: String,
    pub value: Option<ParamValue>,
    pub raw: Option<String>,
    pub key_loc: Loc,
    pub value_loc: Loc,
    pub from_file: Option<FileId>,
}

#[derive(Debug, Clone)]
pub struct NodeInstance {
    pub decl: LaunchNodeDecl,
    pub pkg: Option<String>,
    pub exec: Option<String>,
    pub name: Option<String>,
    pub namespace: String,
    pub fq_name: Option<String>,
    /// (file id, index into `Index::code[file].nodes`)
    pub impls: Vec<(FileId, usize)>,
    pub impl_status: ImplStatus,
    /// Extra parameter declarations from generate_parameter_library specs.
    pub spec_params: Vec<ParamDecl>,
    pub params_open: bool,
    /// Only dotted names (plugin.param) are open: the node loads plugins or uses
    /// generate_parameter_library maps. Top-level names can still be checked.
    pub params_open_dotted: bool,
    pub params: Vec<BoundParam>,
    pub param_sources_complete: bool,
    pub remaps: Vec<(String, String, Loc)>,
    pub enclave: Option<String>,
    pub conditional: bool,
    pub arguments: Vec<Option<String>>,
}

impl NodeInstance {
    pub fn display_name(&self) -> String {
        self.fq_name.clone().unwrap_or_else(|| {
            format!(
                "{}/{}",
                self.pkg.as_deref().unwrap_or("?"),
                self.exec.as_deref().unwrap_or("?")
            )
        })
    }

    /// Effective value of a parameter (last source wins).
    pub fn param(&self, name: &str) -> Option<&BoundParam> {
        self.params.iter().rev().find(|p| p.name == name)
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedEndpoint {
    pub kind: EndpointKind,
    pub name: String,
    pub written: String,
    pub msg_type: Option<String>,
    pub qos: QosSpec,
    pub qos_overrides_enabled: bool,
    pub loc: Loc,
    /// Which remap (if any) changed the name.
    pub remapped_by: Option<Loc>,
}

#[derive(Debug, Clone)]
pub struct EnvSetting {
    pub name: String,
    pub value: Option<String>,
    pub loc: Loc,
}

#[derive(Debug, Clone)]
pub struct RootGraph {
    pub root: FileId,
    pub nodes: Vec<NodeInstance>,
    pub env: Vec<EnvSetting>,
    /// Some launch content could not be interpreted (dynamic code, unresolved includes).
    pub partial: bool,
    /// Every launched file (root + includes).
    pub files: Vec<FileId>,
    /// File references that resolve inside a workspace package but do not exist: (path, loc).
    pub missing_files: Vec<(String, Loc)>,
    /// Included launch files outside the workspace (resolved path text, if known).
    pub external_includes: Vec<String>,
    /// Workspace files (parameter files) this graph read besides `files`, bound or not.
    pub param_deps: Vec<FileId>,
    /// Include edges: (included file, the `<include>` / `IncludeLaunchDescription` that
    /// loaded it). The first edge per file is the path that reached it first.
    pub includes: Vec<(FileId, Loc)>,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub roots: Vec<RootGraph>,
    /// Which (root, node) instances load each parameter file.
    pub file_loads: HashMap<FileId, Vec<(usize, usize)>>,
}

impl Graph {
    pub fn instances(&self) -> impl Iterator<Item = (usize, usize, &NodeInstance)> {
        self.roots
            .iter()
            .enumerate()
            .flat_map(|(r, g)| g.nodes.iter().enumerate().map(move |(n, i)| (r, n, i)))
    }
}

// ============================================================================ value resolution

type Env = HashMap<String, Option<String>>;

/// Resolves a launch value to a string. Paths get `@share:pkg/…` / `@ws:…` markers.
pub fn resolve_lval(v: &LVal, env: &Env) -> Option<String> {
    match v {
        LVal::Lit(s) => Some(s.clone()),
        LVal::Arg(a) => env.get(a).cloned().flatten(),
        LVal::PkgShare(p) => Some(format!("@share:{p}")),
        LVal::PkgPrefix(p) => Some(format!("@prefix:{p}")),
        LVal::Concat(parts) => {
            let mut s = String::new();
            for p in parts {
                s.push_str(&resolve_lval(p, env)?);
            }
            Some(s)
        }
        LVal::PathJoin(parts) => {
            let mut out = String::new();
            for p in parts {
                let s = resolve_lval(p, env)?;
                // An absolute / package-relative piece restarts the path, like os.path.join.
                if out.is_empty() || s.starts_with('/') || s.starts_with('@') {
                    out = s;
                } else {
                    out = format!("{}/{}", out.trim_end_matches('/'), s);
                }
            }
            Some(out)
        }
        LVal::Unknown => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathRes {
    Found(FileId),
    /// Exists on disk but not loaded (not relevant to any extractor).
    Exists(String),
    /// Resolves inside a workspace package but does not exist.
    Missing(String),
    /// Outside the workspace or not statically known.
    Unknown,
}

/// Maps an installed-share path or workspace marker to a workspace file.
pub fn resolve_path(ix: &Index, s: &str) -> PathRes {
    let s = s.replace('\\', "/");
    let (pkg_dir, rest) = if let Some(r) = s
        .strip_prefix("@share:")
        .or_else(|| s.strip_prefix("@prefix:"))
    {
        let (pkg, mut sub) = r.split_once('/').unwrap_or((r, ""));
        let Some(p) = ix.package_by_name(pkg) else {
            // Not in the workspace: maybe an installed package.
            let share_prefix = format!("share/{pkg}/");
            let sub = sub.strip_prefix(share_prefix.as_str()).unwrap_or(sub);
            return match ix.installed.share_path(pkg, sub) {
                Some(abs) if !sub.is_empty() => match ix.sources.find_by_abs(&abs) {
                    Some(id) => PathRes::Found(id),
                    None if abs.exists() => PathRes::Exists(format!("<ros>/{pkg}/{sub}")),
                    None => PathRes::Missing(format!("<ros>/{pkg}/{sub}")),
                },
                _ => PathRes::Unknown,
            };
        };
        // @prefix:pkg/share/pkg/x → x
        let share = format!("share/{pkg}/");
        if let Some(x) = sub.strip_prefix(&share) {
            sub = x;
        }
        (p.dir.clone(), sub.to_string())
    } else if let Some(r) = s.strip_prefix(WS_PREFIX) {
        (String::new(), r.to_string())
    } else {
        return PathRes::Unknown;
    };
    if rest.is_empty() {
        return PathRes::Unknown;
    }
    let rel = if pkg_dir.is_empty() {
        rest.clone()
    } else {
        format!("{pkg_dir}/{rest}")
    };
    let rel = crate::source::normalize_path(std::path::Path::new(&rel));
    if let Some(id) = ix.find_rel(&rel) {
        return PathRes::Found(id);
    }
    if ix.exists_rel(&rel) {
        return PathRes::Exists(rel);
    }
    // Installed layouts sometimes flatten directories: look for a unique file name in the package.
    if !pkg_dir.is_empty() || s.starts_with("@share:") {
        let fname = rest.rsplit('/').next().unwrap_or(&rest);
        let cands: Vec<FileId> = ix
            .files_under(&pkg_dir)
            .filter(|&i| ix.file(i).rel_path.ends_with(&format!("/{fname}")))
            .collect();
        if cands.len() == 1 {
            return PathRes::Found(cands[0]);
        }
        if cands.len() > 1 {
            return PathRes::Unknown;
        }
        if s.starts_with("@share:") || s.starts_with("@prefix:") {
            return PathRes::Missing(rel);
        }
    }
    PathRes::Unknown
}

// ============================================================================ building

struct Scope {
    ns: String,
    /// False once an enclosing namespace could not be evaluated: `ns` is then a guess.
    ns_known: bool,
    params: Vec<(String, Option<String>, Option<ParamValue>, Loc)>,
    remaps: Vec<(Option<String>, Option<String>, Loc)>,
    conditional: bool,
}

/// `link_impl` results for one `build`: the same executable is launched by many launch files,
/// and linking scans the package's code and its dependencies.
type LinkMemo = std::cell::RefCell<
    HashMap<
        (
            Option<String>,
            Option<String>,
            Option<String>,
            LaunchNodeKind,
        ),
        Linked,
    >,
>;

struct Builder<'a> {
    ix: &'a Index,
    links: &'a LinkMemo,
    root: RootGraph,
    stack: Vec<FileId>,
}

pub fn build(ix: &Index) -> Graph {
    build_reusing(ix, None)
}

/// Builds the launch graph, reusing graphs of a previous build of the same workspace whose
/// inputs did not change. `previous` = (graphs of the previous build, changed files). Only
/// valid when node implementations cannot have changed (no code, manifest or build file
/// edits): the caller passes None otherwise.
pub fn build_reusing(ix: &Index, previous: Option<(&[RootGraph], &HashSet<FileId>)>) -> Graph {
    let mut g = Graph::default();
    let mut roots: Vec<FileId> = ix.launches.keys().copied().collect();
    roots.sort();
    let links = LinkMemo::default();
    for r in roots {
        if let Some((prev, changed)) = previous {
            if let Some(old) = prev.iter().find(|x| x.root == r) {
                if !old
                    .files
                    .iter()
                    .chain(old.param_deps.iter())
                    .any(|f| changed.contains(f))
                {
                    g.roots.push(old.clone());
                    continue;
                }
            }
        }
        let mut b = Builder {
            ix,
            links: &links,
            root: RootGraph {
                root: r,
                nodes: Vec::new(),
                env: Vec::new(),
                partial: false,
                files: Vec::new(),
                missing_files: Vec::new(),
                external_includes: Vec::new(),
                param_deps: Vec::new(),
                includes: Vec::new(),
            },
            stack: Vec::new(),
        };
        let mut env = Env::new();
        let scope = Scope {
            ns: "/".into(),
            ns_known: true,
            params: Vec::new(),
            remaps: Vec::new(),
            conditional: false,
        };
        b.launch_file(r, &mut env, scope);
        g.roots.push(b.root);
    }
    for (ri, root) in g.roots.iter().enumerate() {
        for (ni, n) in root.nodes.iter().enumerate() {
            let mut seen = HashSet::new();
            for p in &n.params {
                if let Some(f) = p.from_file {
                    if seen.insert(f) {
                        g.file_loads.entry(f).or_default().push((ri, ni));
                    }
                }
            }
        }
    }
    g
}

impl<'a> Builder<'a> {
    fn launch_file(&mut self, file: FileId, env: &mut Env, scope: Scope) {
        if self.stack.contains(&file) || self.stack.len() > 12 {
            self.root.partial = true;
            return;
        }
        let Some(lf) = self.ix.launches.get(&file) else {
            self.root.partial = true;
            return;
        };
        if lf.partial {
            self.root.partial = true;
        }
        self.root.files.push(file);
        self.stack.push(file);
        let mut scope = scope;
        self.items(&lf.items, env, &mut scope);
        self.stack.pop();
    }

    fn items(&mut self, items: &[LaunchItem], env: &mut Env, scope: &mut Scope) {
        for it in items {
            match it {
                LaunchItem::Arg { name, default, .. } => {
                    // A value passed by an includer (or the command line) wins over the default.
                    if !env.contains_key(name) {
                        let v = default.as_ref().and_then(|d| resolve_lval(d, env));
                        env.insert(name.clone(), v);
                    }
                }
                LaunchItem::SetParam { name, value, loc } => {
                    let v = resolve_lval(value, env);
                    let typed = v
                        .as_deref()
                        .map(|s| crate::extract::yaml::scalar_value(s, false));
                    scope.params.push((name.clone(), v, typed, *loc));
                }
                LaunchItem::SetRemap { from, to, loc } => {
                    scope
                        .remaps
                        .push((resolve_lval(from, env), resolve_lval(to, env), *loc));
                }
                LaunchItem::SetEnv { name, value, loc } => {
                    self.root.env.push(EnvSetting {
                        name: name.clone(),
                        value: resolve_lval(value, env),
                        loc: *loc,
                    });
                }
                LaunchItem::Group {
                    namespace,
                    items,
                    scoped,
                    ..
                } => {
                    let ns_piece = namespace.as_ref().map(|n| resolve_lval(n, env));
                    if let Some(None) = ns_piece {
                        self.root.partial = true;
                    }
                    if items.is_empty() && !scoped {
                        // PushRosNamespace: applies to the rest of the current scope.
                        match ns_piece {
                            Some(Some(p)) => {
                                scope.ns_known = scope.ns_known || p.trim().starts_with('/');
                                scope.ns = join_ns(&scope.ns, &p);
                            }
                            Some(None) => scope.ns_known = false,
                            None => {}
                        }
                        continue;
                    }
                    let mut inner = Scope {
                        ns: match &ns_piece {
                            Some(Some(p)) => join_ns(&scope.ns, p),
                            _ => scope.ns.clone(),
                        },
                        ns_known: match &ns_piece {
                            Some(Some(p)) => scope.ns_known || p.trim().starts_with('/'),
                            Some(None) => false,
                            None => scope.ns_known,
                        },
                        params: scope.params.clone(),
                        remaps: scope.remaps.clone(),
                        conditional: scope.conditional,
                    };
                    if *scoped {
                        let mut env2 = env.clone();
                        self.items(items, &mut env2, &mut inner);
                    } else {
                        self.items(items, env, &mut inner);
                        scope.params = inner.params;
                        scope.remaps = inner.remaps;
                    }
                }
                LaunchItem::Include {
                    path,
                    args,
                    conditional,
                    loc,
                } => {
                    let p = resolve_lval(path, env);
                    let mut env2 = env.clone();
                    for (k, v) in args {
                        env2.insert(k.clone(), resolve_lval(v, env));
                    }
                    match p.as_deref().map(|p| resolve_path(self.ix, p)) {
                        Some(PathRes::Found(id)) if self.ix.launches.contains_key(&id) => {
                            let inner = Scope {
                                ns: scope.ns.clone(),
                                ns_known: scope.ns_known,
                                params: scope.params.clone(),
                                remaps: scope.remaps.clone(),
                                conditional: scope.conditional || *conditional,
                            };
                            self.root.includes.push((id, *loc));
                            self.launch_file(id, &mut env2, inner);
                        }
                        Some(PathRes::Missing(rel)) => {
                            self.root.missing_files.push((rel, *loc));
                            self.root.partial = true;
                        }
                        _ => {
                            self.root.partial = true;
                            self.root
                                .external_includes
                                .push(p.clone().unwrap_or_default());
                        }
                    }
                }
                LaunchItem::Node(d) => {
                    let inst = self.instance(d, env, scope);
                    self.root.nodes.push(inst);
                }
            }
        }
    }

    fn instance(&mut self, d: &LaunchNodeDecl, env: &Env, scope: &Scope) -> NodeInstance {
        let ix = self.ix;
        let pkg = resolve_lval(&d.pkg, env);
        let exec = resolve_lval(&d.exec, env);
        let plugin = d.plugin.as_ref().and_then(|p| resolve_lval(p, env));
        let key = (pkg.clone(), exec.clone(), plugin.clone(), d.kind);
        let cached = self.links.borrow().get(&key).cloned();
        let linked = cached.unwrap_or_else(|| {
            let l = link_impl(
                ix,
                pkg.as_deref(),
                exec.as_deref(),
                plugin.as_deref(),
                d.kind,
            );
            self.links.borrow_mut().insert(key, l.clone());
            l
        });
        let (impls, status, spec_params, mut open, open_dotted) = linked;
        let (ns, ns_known) = match &d.namespace {
            Some(n) => match resolve_lval(n, env) {
                Some(n) => (
                    join_ns(&scope.ns, &n),
                    scope.ns_known || n.trim().starts_with('/'),
                ),
                None => {
                    self.root.partial = true;
                    (scope.ns.clone(), false)
                }
            },
            None => (scope.ns.clone(), scope.ns_known),
        };
        let name = match &d.name {
            Some(n) => resolve_lval(n, env),
            None => {
                let names: HashSet<&str> = impls
                    .iter()
                    .filter_map(|(f, i)| ix.code[f].nodes[*i].default_name.as_deref())
                    .collect();
                if names.len() == 1 {
                    names.into_iter().next().map(|s| s.to_string())
                } else {
                    None
                }
            }
        };
        // A guessed namespace gives a guessed name: leave it unknown.
        let fq_name = name.as_ref().filter(|_| ns_known).map(|n| join_ns(&ns, n));
        // ---------------------------------------------------------- parameters
        let mut params = Vec::new();
        let mut complete = true;
        for (n, v, typed, loc) in &scope.params {
            params.push(BoundParam {
                name: n.clone(),
                value: typed.clone(),
                raw: v.clone(),
                key_loc: *loc,
                value_loc: *loc,
                from_file: None,
            });
        }
        for src in &d.params {
            match src {
                LaunchParamSource::Inline {
                    name,
                    value,
                    typed,
                    loc,
                } => {
                    let raw = resolve_lval(value, env);
                    let value = typed.clone().or_else(|| {
                        raw.as_deref()
                            .map(|s| crate::extract::yaml::scalar_value(s, false))
                    });
                    params.push(BoundParam {
                        name: name.clone(),
                        value,
                        raw,
                        key_loc: *loc,
                        value_loc: *loc,
                        from_file: None,
                    });
                }
                LaunchParamSource::Opaque { .. } => complete = false,
                LaunchParamSource::File {
                    path,
                    root_key,
                    loc,
                } => {
                    let p = resolve_lval(path, env);
                    let rk = root_key.as_ref().map(|r| resolve_lval(r, env));
                    let found = p.as_deref().map(|p| resolve_path(ix, p));
                    if let Some(PathRes::Found(fid)) = &found {
                        self.root.param_deps.push(*fid);
                    }
                    match found {
                        Some(PathRes::Found(fid)) => match ix.param_files.get(&fid) {
                            Some(pf) => {
                                let prefix = match &rk {
                                    Some(Some(k)) if !k.is_empty() => Some(k.clone()),
                                    Some(None) => {
                                        // RewrittenYaml with an unknown root key: be lenient.
                                        complete = false;
                                        None
                                    }
                                    _ => None,
                                };
                                // A key matches this node; with the namespace unknown, only keys that
                                // match in any namespace (`/**`, `/**/name`) are certain.
                                let matches_node = |k: &str| -> Option<bool> {
                                    match (&fq_name, &name) {
                                        (Some(fq), _) => Some(key_matches(k, fq)),
                                        (None, Some(n)) => {
                                            let a = key_matches(k, &join_ns("/", n));
                                            let b = key_matches(
                                                k,
                                                &join_ns("/rosfix_ns_a/rosfix_ns_b", n),
                                            );
                                            (a == b).then_some(a)
                                        }
                                        (None, None) => None,
                                    }
                                };
                                if fq_name.is_some() || name.is_some() {
                                    for s in &pf.sections {
                                        let key = match &prefix {
                                            Some(pre) => format!(
                                                "{}/{}",
                                                pre.trim_matches('/'),
                                                s.node_key.trim_start_matches('/')
                                            ),
                                            None => s.node_key.clone(),
                                        };
                                        let wild =
                                            format!("**/{}", s.node_key.trim_start_matches('/'));
                                        let hit = |k: &str| matches_node(k).unwrap_or(false);
                                        if matches_node(&key).is_none() {
                                            // Depends on the namespace, which is unknown.
                                            complete = false;
                                            continue;
                                        }
                                        if hit(&key) || (matches!(rk, Some(None)) && hit(&wild)) {
                                            for e in &s.params {
                                                params.push(BoundParam {
                                                    name: e.name.clone(),
                                                    value: Some(e.value.clone()),
                                                    raw: e.raw.clone(),
                                                    key_loc: e.key_loc,
                                                    value_loc: e.value_loc,
                                                    from_file: Some(fid),
                                                });
                                            }
                                        }
                                    }
                                } else {
                                    complete = false;
                                }
                                // Record the load even if nothing matched.
                                params.push(BoundParam {
                                    name: String::from("\u{0}load"),
                                    value: None,
                                    raw: None,
                                    key_loc: *loc,
                                    value_loc: *loc,
                                    from_file: Some(fid),
                                });
                            }
                            None => complete = false,
                        },
                        Some(PathRes::Missing(rel)) => {
                            self.root.missing_files.push((rel, *loc));
                            complete = false;
                        }
                        _ => complete = false,
                    }
                }
            }
        }
        // ---------------------------------------------------------- remaps
        let node_name = name.clone().unwrap_or_default();
        let mut remaps = Vec::new();
        for (f, t, loc) in &d.remaps {
            if let (Some(f), Some(t)) = (resolve_lval(f, env), resolve_lval(t, env)) {
                remaps.push((
                    expand_name(&f, &ns, &node_name),
                    expand_name(&t, &ns, &node_name),
                    *loc,
                ));
            }
        }
        for (f, t, loc) in &scope.remaps {
            if let (Some(f), Some(t)) = (f, t) {
                remaps.push((
                    expand_name(f, &ns, &node_name),
                    expand_name(t, &ns, &node_name),
                    *loc,
                ));
            }
        }
        if impls.is_empty() {
            open = true;
        }
        NodeInstance {
            decl: d.clone(),
            pkg,
            exec,
            name,
            namespace: ns,
            fq_name,
            impls,
            impl_status: status,
            spec_params,
            params_open: open,
            params_open_dotted: open_dotted,
            params,
            param_sources_complete: complete,
            remaps,
            enclave: d.enclave.as_ref().and_then(|e| resolve_lval(e, env)),
            conditional: d.conditional || scope.conditional,
            arguments: d.arguments.iter().map(|a| resolve_lval(a, env)).collect(),
        }
    }
}

pub type Linked = (Vec<(FileId, usize)>, ImplStatus, Vec<ParamDecl>, bool, bool);

/// Links a launched (pkg, exec | plugin) to node implementations in the workspace.
pub fn link_impl(
    ix: &Index,
    pkg: Option<&str>,
    exec: Option<&str>,
    plugin: Option<&str>,
    kind: LaunchNodeKind,
) -> Linked {
    let Some(pkg_name) = pkg else {
        return (Vec::new(), ImplStatus::Unresolved, Vec::new(), true, true);
    };
    let Some(p) = ix.package_by_name(pkg_name) else {
        // Not in the workspace: use the knowledge pack for this distribution if it knows the node.
        let key = if kind == LaunchNodeKind::Composable {
            plugin
        } else {
            exec
        };
        if let Some((impls, open, dotted)) =
            key.and_then(|k| ix.pack_links.get(&(pkg_name.to_string(), k.to_string())))
        {
            return (
                impls.clone(),
                ImplStatus::Resolved,
                Vec::new(),
                *open,
                *dotted,
            );
        }
        return (Vec::new(), ImplStatus::External, Vec::new(), true, true);
    };
    let mut files: Vec<FileId> = Vec::new();
    let mut classes: Vec<String> = Vec::new();
    if kind == LaunchNodeKind::Composable {
        if let Some(cls) = plugin {
            classes.push(cls.rsplit("::").next().unwrap_or(cls).to_string());
        }
    } else if let Some(e) = exec {
        if let Some(x) = p.executables.iter().find(|x| x.name == e) {
            for s in &x.sources {
                if let Some(id) = ix.find_rel(s) {
                    files.push(id);
                }
            }
        }
    }
    let mut impls: Vec<(FileId, usize)> = Vec::new();
    for &f in &files {
        if let Some(c) = ix.code.get(&f) {
            for (i, n) in c.nodes.iter().enumerate() {
                impls.push((f, i));
                if let Some(cn) = &n.class_name {
                    classes.push(cn.clone());
                }
            }
        }
    }
    // Pull in same-class impls from other files of the package (headers, split sources).
    let pkg_idx = ix.packages.iter().position(|q| q.name == p.name);
    for (fid, c) in &ix.code {
        if ix.file_pkg.get(*fid).copied().flatten() != pkg_idx {
            continue;
        }
        for (i, n) in c.nodes.iter().enumerate() {
            if n.class_name.as_ref().is_some_and(|cn| classes.contains(cn))
                && !impls.contains(&(*fid, i))
            {
                impls.push((*fid, i));
            }
        }
    }
    // A main() that only spins a node class: drop empty free impls when a class impl exists.
    let has_class = impls
        .iter()
        .any(|(f, i)| ix.code[f].nodes[*i].class_name.is_some());
    if has_class {
        impls.retain(|(f, i)| {
            let n = &ix.code[f].nodes[*i];
            n.class_name.is_some() || !n.endpoints.is_empty() || !n.params.is_empty()
        });
    }
    impls.sort();
    impls.dedup();
    let mut open = impls.iter().any(|(f, i)| ix.code[f].nodes[*i].params_open);
    let touched: HashSet<FileId> = impls
        .iter()
        .map(|(f, _)| *f)
        .chain(files.iter().copied())
        .collect();
    let mut open_dotted = touched
        .iter()
        .any(|f| ix.code.get(f).is_some_and(|c| c.loads_plugins));
    let mut spec_params = Vec::new();
    // Declarations made through a node handle anywhere in the package: attribute them to a
    // linked node class, otherwise we cannot know which node receives them.
    for (fid, c) in &ix.code {
        if ix.file_pkg.get(*fid).copied().flatten() != pkg_idx {
            continue;
        }
        for (cls, decl) in &c.class_params {
            let linked = cls.as_ref().is_some_and(|x| classes.contains(x));
            match decl {
                // Literal names declared by helpers are accepted as declared for this node.
                Some(
                    d @ ParamDecl {
                        name: NamePattern::Exact(_),
                        ..
                    },
                ) => spec_params.push(d.clone()),
                // `prefix + ".x"` (plugins, per-joint maps): only dotted names become uncertain.
                Some(
                    d @ ParamDecl {
                        name: NamePattern::Suffix(_),
                        ..
                    },
                ) => {
                    spec_params.push(d.clone());
                    if !linked {
                        open_dotted = true;
                    }
                }
                None if linked => open = true,
                None => open_dotted = true,
            }
        }
    }
    let mut lib_params: Vec<ParamDecl> = Vec::new();
    for s in &p.param_lib_specs {
        if let Some((d, o)) = ix.param_specs.get(s) {
            lib_params.extend(d.iter().cloned());
            open_dotted |= *o;
        }
    }
    // `ParamListener(node, "ns")` declares the library's parameters as `ns.<name>`.
    for f in &touched {
        for prefix in ix
            .code
            .get(f)
            .map(|c| c.param_prefixes.as_slice())
            .unwrap_or_default()
        {
            match prefix {
                Some(pre) if !pre.is_empty() => spec_params.extend(lib_params.iter().map(|d| {
                    let mut d = d.clone();
                    if let NamePattern::Exact(n) = &d.name {
                        d.name = NamePattern::Exact(format!("{pre}.{n}"));
                    }
                    d
                })),
                Some(_) => {}
                None => open_dotted = true,
            }
        }
    }
    spec_params.extend(lib_params);
    // Library code in workspace dependencies (e.g. nav2_ros_common's node utilities) may declare
    // parameters on every node that uses it: accept their literal declarations.
    let deps = dependency_closure(ix, p);
    let mut dep_classes: HashSet<&str> = HashSet::new();
    for (fid, c) in &ix.code {
        let Some(pi) = ix.file_pkg.get(*fid).copied().flatten() else {
            continue;
        };
        // Test code of a dependency (e.g. nav2_util/test/test_node_utils.cpp declaring `Foo`)
        // declares nothing on the nodes that use the library.
        if !deps.contains(&pi) || is_test_rel(&ix.file(*fid).rel_path) {
            continue;
        }
        dep_classes.extend(c.classes_defined.iter().map(|s| s.as_str()));
        for (_, d) in &c.class_params {
            if let Some(
                d @ ParamDecl {
                    name: NamePattern::Exact(_),
                    ..
                },
            ) = d
            {
                spec_params.push(d.clone());
            }
        }
        for n in &c.nodes {
            spec_params.extend(
                n.params
                    .iter()
                    .filter(|d| matches!(d.name, NamePattern::Exact(_)))
                    .cloned(),
            );
        }
    }
    // The same for generate_parameter_library specs of dependencies: a library class (e.g.
    // MoveIt's smoothing plugins in moveit_core) constructs its `ParamListener(node)` on the
    // hosting node, which then declares the spec's parameters, under any `ParamListener(node,
    // "ns")` prefix the dependency's code uses.
    for &pi in &deps {
        if Some(pi) == pkg_idx {
            continue;
        }
        let Some(dp) = ix.packages.get(pi) else {
            continue;
        };
        let decls: Vec<&ParamDecl> = dp
            .param_lib_specs
            .iter()
            .filter_map(|s| ix.param_specs.get(s))
            .flat_map(|(d, _)| d.iter())
            .filter(|d| matches!(d.name, NamePattern::Exact(_)))
            .collect();
        if decls.is_empty() {
            continue;
        }
        let mut prefixes: Vec<&str> = ix
            .code
            .iter()
            .filter(|(fid, _)| ix.file_pkg.get(**fid).copied().flatten() == Some(pi))
            .flat_map(|(_, c)| c.param_prefixes.iter().flatten())
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        for d in decls {
            spec_params.push(d.clone());
            if let NamePattern::Exact(n) = &d.name {
                for pre in &prefixes {
                    let mut d = d.clone();
                    d.name = NamePattern::Exact(format!("{pre}.{n}"));
                    spec_params.push(d);
                }
            }
        }
    }
    // A node class deriving from a non-standard base we cannot see may inherit any parameters.
    for (f, i) in &impls {
        if let Some(b) = &ix.code[f].nodes[*i].base {
            let last = b
                .split('<')
                .next()
                .unwrap_or(b)
                .rsplit("::")
                .next()
                .unwrap_or(b)
                .rsplit('.')
                .next()
                .unwrap_or(b)
                .trim();
            let standard = matches!(
                b.trim(),
                "Node"
                    | "rclcpp::Node"
                    | "rclcpp_lifecycle::LifecycleNode"
                    | "rclpy.node.Node"
                    | "LifecycleNode"
            ) && !b.contains("nav2");
            if !standard && !dep_classes.contains(last) {
                open = true;
            }
        }
    }
    let status = if impls.is_empty() {
        ImplStatus::Unresolved
    } else {
        ImplStatus::Resolved
    };
    if !p.executables_complete && impls.is_empty() {
        open = true;
    }
    (impls, status, spec_params, open, open_dotted)
}

/// A file under a `test/` or `tests/` directory.
fn is_test_rel(rel: &str) -> bool {
    rel.split('/').any(|seg| seg == "test" || seg == "tests")
}

/// Workspace packages a package depends on (build or run), transitively, including itself.
fn dependency_closure(ix: &Index, p: &Package) -> HashSet<usize> {
    let mut out = HashSet::new();
    let mut stack = vec![p.name.as_str()];
    while let Some(n) = stack.pop() {
        let Some(i) = ix.packages.iter().position(|q| q.name == n) else {
            continue;
        };
        if !out.insert(i) {
            continue;
        }
        for d in &ix.packages[i].deps {
            if !matches!(d.kind, DepKind::Test | DepKind::BuildTool) {
                stack.push(d.name.as_str());
            }
        }
    }
    out
}

/// Endpoints of an instance with names expanded and remapped.
pub fn instance_endpoints(ix: &Index, n: &NodeInstance) -> (Vec<ResolvedEndpoint>, bool) {
    let mut out = Vec::new();
    let mut complete = n.impl_status == ImplStatus::Resolved;
    let node_name = n.name.clone().unwrap_or_default();
    if n.name.is_none() {
        complete = false;
    }
    for (f, i) in &n.impls {
        let imp = &ix.code[f].nodes[*i];
        for e in &imp.endpoints {
            let Some(written) = &e.name else {
                complete = false;
                continue;
            };
            if n.name.is_none() && written.starts_with('~') {
                complete = false;
                continue;
            }
            let mut name = expand_name(written, &n.namespace, &node_name);
            let mut remapped_by = None;
            for (from, to, loc) in &n.remaps {
                if *from == name {
                    name = to.clone();
                    remapped_by = Some(*loc);
                    break;
                }
            }
            out.push(ResolvedEndpoint {
                kind: e.kind,
                name,
                written: written.clone(),
                msg_type: e.msg_type.clone(),
                qos: e.qos.clone(),
                qos_overrides_enabled: e.qos_overrides_enabled,
                loc: e.loc,
                remapped_by,
            });
        }
    }
    (out, complete)
}

/// Collects every parameter the node's implementation declares.
pub fn declared_params<'i>(ix: &'i Index, n: &'i NodeInstance) -> Vec<&'i ParamDecl> {
    let mut v: Vec<&ParamDecl> = n
        .impls
        .iter()
        .flat_map(|(f, i)| ix.code[f].nodes[*i].params.iter())
        .collect();
    v.extend(n.spec_params.iter());
    v
}

/// True for the synthetic marker recording that a file was loaded.
pub fn is_load_marker(p: &BoundParam) -> bool {
    p.name.starts_with('\u{0}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(join_ns("/", "r1"), "/r1");
        assert_eq!(join_ns("/a", "b"), "/a/b");
        assert_eq!(join_ns("/a", "/b"), "/b");
        assert_eq!(expand_name("scan", "/r1", "n"), "/r1/scan");
        assert_eq!(expand_name("/scan", "/r1", "n"), "/scan");
        assert_eq!(expand_name("~/out", "/r1", "n"), "/r1/n/out");
    }

    #[test]
    fn rcl_key_matching() {
        assert!(key_matches("controller_server", "/controller_server"));
        assert!(!key_matches(
            "controller_server",
            "/robot1/controller_server"
        ));
        assert!(key_matches("/**", "/robot1/controller_server"));
        assert!(key_matches(
            "/**/controller_server",
            "/robot1/controller_server"
        ));
        assert!(key_matches(
            "robot1/controller_server",
            "/robot1/controller_server"
        ));
        assert!(key_matches("/*/amcl", "/r2/amcl"));
        assert!(!key_matches("/*/amcl", "/amcl"));
        assert!(key_matches("**", "/a/b"));
    }
}
