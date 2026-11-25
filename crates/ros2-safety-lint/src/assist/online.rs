//! `--online`: background references for findings, fetched from an allowlist of public
//! sources by a separate helper process.
//!
//! Privacy and isolation:
//! - Queries are built **only from the rule** that fired (e.g. "ros2 parameter not declared
//!   ignored yaml"). No file contents, paths, node or package names leave the machine.
//! - The analysing process never opens a network connection. It starts itself again as a
//!   helper (`rosfix __fetch`), passes the queries on stdin and reads results from stdout.
//! - The helper talks only HTTPS to allowlisted hosts (plus a user-configured SearXNG), with
//!   no redirects, a 5 s timeout per request and a 256 KiB response cap. Fetched text is
//!   reduced to titles and links and is only ever shown to the user — it never feeds edits.
//! - Results are cached on disk, so repeated runs stay offline.

use crate::engine::Analysis;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct FetchRequest {
    pub queries: Vec<String>,
    pub searxng_url: Option<String>,
    /// Text documents to download (the rosdistro index, see `crate::fetch`).
    #[serde(default)]
    pub texts: Vec<String>,
    /// Source archives to download and unpack (see `crate::fetch`).
    #[serde(default)]
    pub tarballs: Vec<TarballRequest>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TarballRequest {
    pub url: String,
    /// Directory the wanted files are unpacked into.
    pub dest: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct TarballResult {
    pub files: usize,
    /// Commit recorded in the archive (GitHub archives carry it in a pax header).
    pub commit: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct FetchResponse {
    /// query → [(title, url)]
    pub results: BTreeMap<String, Vec<(String, String)>>,
    pub errors: Vec<String>,
    /// url → text
    #[serde(default)]
    pub texts: BTreeMap<String, String>,
    /// url → result
    #[serde(default)]
    pub tarballs: BTreeMap<String, TarballResult>,
}

/// Search phrase per rule. Purely generic: nothing from the user's workspace.
pub fn query_for_rule(rule: &str) -> Option<&'static str> {
    Some(match rule {
        "P001" => "ros2 parameter yaml node name namespace not applied",
        "P002" => "ros2 yaml ros__parameters missing parameters not loaded",
        "P003" => "ros2 parameter not declared yaml value ignored",
        "P004" => "ros2 parameter invalid type expected double got integer",
        "P005" => "ros2 declare_parameter no default parameter not set exception",
        "T001" | "T005" => "ros2 subscriber not receiving messages namespace topic name",
        "T002" => "ros2 launch remapping has no effect",
        "T003" => "ros2 topic different message types",
        "E001" => "ros2 call service inside callback deadlock callback group",
        "E002" => "ros2 sleep inside callback blocks executor",
        "C001" | "C005" => "ros2_control controller joint not found in ros2_control tag",
        "C002" => "ros2_control command interface does not exist controller",
        "C003" => "ros2_control controller type not found pluginlib",
        "C006" => "ros2_control hardware plugin missing",
        "L001" => "ros2 launch executable not found libexec directory",
        "L002" => "package.xml missing dependency rosdep",
        "L003" => "ros2 python package setup.cfg install_scripts executable not found",
        "L004" => "ros2 launch file not found install share directory",
        "L005" => "ros2 launch parameter file not found",
        "S001" | "S002" | "S003" => "ros2 use_sim_time extrapolation into the future",
        "Q001" => "ros2 qos incompatible reliability best effort reliable no messages",
        "Q002" => "ros2 transient_local volatile qos incompatible",
        "Q004" => "ros2 transient local latched topic late subscriber",
        "Q005" => "ros2 qos_overrides parameters not applied",
        "N001" | "N002" => "nav2 plugin not found pluginlib params",
        "N003" => "nav2 frame does not exist tf",
        "N004" | "N005" => "nav2 costmap footprint robot_radius inflation_radius",
        "N006" => "nav2 parameter name",
        "X001" | "X002" | "X003" | "X004" | "X005" | "X006" => "sros2 security enclave permissions",
        _ => return None,
    })
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("rosfix").join("online-cache.json"))
}

fn load_cache() -> HashMap<String, Vec<(String, String)>> {
    cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_cache(c: &HashMap<String, Vec<(String, String)>>) {
    if let Some(p) = cache_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
        let _ = std::fs::write(p, serde_json::to_string(c).unwrap_or_default());
    }
}

/// Looks up references for the analysis' findings and attaches them. Returns the queries
/// that were sent (for `--show-queries`) and any errors.
pub fn lookup(
    a: &mut Analysis,
    searxng_url: Option<String>,
    show_queries: bool,
) -> Result<usize, String> {
    let mut wanted: Vec<String> = a
        .diagnostics
        .iter()
        .filter_map(|d| query_for_rule(d.rule))
        .map(|s| s.to_string())
        .collect();
    wanted.sort();
    wanted.dedup();
    let mut cache = load_cache();
    let missing: Vec<String> = wanted
        .iter()
        .filter(|q| !cache.contains_key(*q))
        .cloned()
        .collect();
    if !missing.is_empty() {
        if show_queries {
            for q in &missing {
                eprintln!("rosfix: online query: {q}");
            }
        }
        let resp = run_helper(
            &FetchRequest {
                queries: missing,
                searxng_url,
                ..Default::default()
            },
            Duration::from_secs(30),
        )?;
        for e in &resp.errors {
            eprintln!("rosfix: online: {e}");
        }
        for (q, r) in resp.results {
            cache.insert(q, r);
        }
        save_cache(&cache);
    }
    let mut n = 0;
    for d in &mut a.diagnostics {
        if let Some(q) = query_for_rule(d.rule) {
            if let Some(refs) = cache.get(q) {
                d.references = refs.iter().take(3).cloned().collect();
                n += usize::from(!d.references.is_empty());
            }
        }
    }
    Ok(n)
}

/// Starts `rosfix __fetch` and exchanges JSON over stdin/stdout, with an overall deadline.
pub fn run_helper(req: &FetchRequest, timeout: Duration) -> Result<FetchResponse, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg("__fetch")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env(
            "SYSTEMROOT",
            std::env::var_os("SYSTEMROOT").unwrap_or_default(),
        )
        .spawn()
        .map_err(|e| format!("starting the online helper: {e}"))?;
    let body = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(&body)
        .map_err(|e| e.to_string())?;
    // Read stdout while the helper runs: a response larger than the pipe buffer (the
    // rosdistro index) would otherwise block the helper until the deadline.
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        stdout.read_to_string(&mut out).map(|_| out)
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("online helper timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = reader
        .join()
        .map_err(|_| "online helper reader failed".to_string())?
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&out).map_err(|e| format!("online helper returned invalid data: {e}"))
}

// ============================================================================ helper process

/// Entry point of `rosfix __fetch` (the only code that touches the network).
pub fn helper_main() -> i32 {
    let mut input = String::new();
    if std::io::stdin()
        .take(64 * 1024)
        .read_to_string(&mut input)
        .is_err()
    {
        return 2;
    }
    let Ok(req) = serde_json::from_str::<FetchRequest>(&input) else {
        return 2;
    };
    let resp = fetch_all(&req);
    let _ =
        std::io::stdout().write_all(serde_json::to_string(&resp).unwrap_or_default().as_bytes());
    0
}

const ALLOWED_HOSTS: &[&str] = &["api.stackexchange.com", "api.github.com"];
const GITHUB_ORGS: &str = "+org:ros2+org:ros+org:ros-navigation+org:ros-controls+org:moveit+org:ros-planning+org:ament+org:ros-tooling+org:gazebosim";
const MAX_QUERIES: usize = 12;

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    rest.split(['/', ':', '?']).next()
}

/// The allowlist check every request goes through.
pub fn allowed(url: &str, searxng: Option<&str>) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    if url.starts_with("https://") && ALLOWED_HOSTS.contains(&host) {
        return true;
    }
    // Sources for local knowledge packs: the rosdistro index and GitHub source archives.
    let path = &url[url.find(host).map_or(0, |i| i + host.len())..];
    if url.starts_with("https://")
        && ((host == "raw.githubusercontent.com" && path.starts_with("/ros/rosdistro/"))
            || (host == "codeload.github.com" && path.contains("/tar.gz/")))
    {
        return true;
    }
    // A user-configured SearXNG instance (http allowed only for localhost).
    if let Some(sx) = searxng.and_then(host_of) {
        if host == sx && (url.starts_with("https://") || matches!(host, "localhost" | "127.0.0.1"))
        {
            return true;
        }
    }
    false
}

#[cfg(feature = "online")]
fn get_json(url: &str, searxng: Option<&str>) -> Result<serde_json::Value, String> {
    if !allowed(url, searxng) {
        return Err(format!("blocked (not allowlisted): {url}"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .max_redirects(0)
        .https_only(!url.starts_with("http://"))
        .user_agent("rosfix")
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| e.to_string())?;
    let text = resp
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

#[cfg(not(feature = "online"))]
fn get_json(_url: &str, _searxng: Option<&str>) -> Result<serde_json::Value, String> {
    Err("this rosfix build has no online support (feature `online`)".into())
}

#[cfg(feature = "online")]
/// Largest rosdistro index accepted (Rolling's is about 1.5 MiB).
const MAX_TEXT: u64 = 16 * 1024 * 1024;
#[cfg(feature = "online")]
/// Largest compressed source archive accepted.
const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;

#[cfg(feature = "online")]
fn source_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .https_only(true)
        .user_agent("rosfix")
        .build()
        .into()
}

#[cfg(feature = "online")]
fn get_text(url: &str) -> Result<String, String> {
    if !allowed(url, None) {
        return Err(format!("blocked (not allowlisted): {url}"));
    }
    let mut resp = source_agent(Duration::from_secs(60))
        .get(url)
        .call()
        .map_err(|e| e.to_string())?;
    resp.body_mut()
        .with_config()
        .limit(MAX_TEXT)
        .read_to_string()
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "online"))]
fn get_text(_url: &str) -> Result<String, String> {
    Err("this rosfix build has no online support (feature `online`)".into())
}

#[cfg(feature = "online")]
fn get_tarball(t: &TarballRequest) -> TarballResult {
    let run = || -> Result<TarballResult, String> {
        if !allowed(&t.url, None) {
            return Err(format!("blocked (not allowlisted): {}", t.url));
        }
        let mut resp = source_agent(Duration::from_secs(300))
            .get(&t.url)
            .call()
            .map_err(|e| e.to_string())?;
        let reader = resp.body_mut().with_config().limit(MAX_ARCHIVE).reader();
        std::fs::create_dir_all(&t.dest).map_err(|e| e.to_string())?;
        let (files, commit) = crate::fetch::extract_tar_gz(reader, &t.dest, &crate::fetch::LIMITS)?;
        Ok(TarballResult {
            files,
            commit,
            error: None,
        })
    };
    run().unwrap_or_else(|e| TarballResult {
        error: Some(e),
        ..Default::default()
    })
}

#[cfg(not(feature = "online"))]
fn get_tarball(_t: &TarballRequest) -> TarballResult {
    TarballResult {
        error: Some("this rosfix build has no online support (feature `online`)".into()),
        ..Default::default()
    }
}

fn fetch_all(req: &FetchRequest) -> FetchResponse {
    let mut resp = FetchResponse::default();
    for url in req.texts.iter().take(4) {
        match get_text(url) {
            Ok(t) => {
                resp.texts.insert(url.clone(), t);
            }
            Err(e) => resp.errors.push(format!("{url}: {e}")),
        }
    }
    for t in req.tarballs.iter().take(16) {
        resp.tarballs.insert(t.url.clone(), get_tarball(t));
    }
    let sx = req.searxng_url.as_deref();
    for q in req.queries.iter().take(MAX_QUERIES) {
        let qe = url_encode(q);
        // One list per source, interleaved below so that every source reaches the top three
        // references a finding shows (appending would hide everything after the first source).
        let mut sources: Vec<Vec<(String, String)>> = Vec::new();
        let mut refs: Vec<(String, String)> = Vec::new();
        // `similar` ranks questions by title similarity (search/advanced needs every word to match).
        let se = format!("https://api.stackexchange.com/2.3/similar?order=desc&sort=relevance&title={qe}&site=robotics&pagesize=3");
        match get_json(&se, sx) {
            Ok(v) => {
                for it in v["items"].as_array().into_iter().flatten() {
                    if let (Some(t), Some(u)) = (it["title"].as_str(), it["link"].as_str()) {
                        refs.push((html_unescape(t), u.to_string()));
                    }
                }
            }
            Err(e) => resp.errors.push(format!("stackexchange: {e}")),
        }
        sources.push(std::mem::take(&mut refs));
        // Only issues in the official ROS organisations (multiple org: qualifiers are OR-ed).
        let gh =
            format!("https://api.github.com/search/issues?q={qe}+is:issue{GITHUB_ORGS}&per_page=2");
        match get_json(&gh, sx) {
            Ok(v) => {
                for it in v["items"].as_array().into_iter().flatten() {
                    if let (Some(t), Some(u)) = (it["title"].as_str(), it["html_url"].as_str()) {
                        refs.push((t.to_string(), u.to_string()));
                    }
                }
            }
            Err(e) => resp.errors.push(format!("github: {e}")),
        }
        sources.push(std::mem::take(&mut refs));
        if let Some(base) = sx {
            let url = format!("{}/search?q={qe}&format=json", base.trim_end_matches('/'));
            match get_json(&url, sx) {
                Ok(v) => {
                    for it in v["results"].as_array().into_iter().flatten().take(3) {
                        if let (Some(t), Some(u)) = (it["title"].as_str(), it["url"].as_str()) {
                            refs.push((t.to_string(), u.to_string()));
                        }
                    }
                }
                Err(e) => resp.errors.push(format!("searxng: {e}")),
            }
            sources.push(std::mem::take(&mut refs));
        }
        let longest = sources.iter().map(Vec::len).max().unwrap_or(0);
        for i in 0..longest {
            refs.extend(sources.iter().filter_map(|s| s.get(i).cloned()));
        }
        refs.truncate(5);
        resp.results.insert(q.clone(), refs);
    }
    resp.errors.sort();
    resp.errors.dedup();
    resp
}

fn html_unescape(s: &str) -> String {
    s.replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        assert!(allowed("https://api.stackexchange.com/2.3/search", None));
        assert!(!allowed("http://api.stackexchange.com/2.3/search", None));
        assert!(!allowed("https://evil.example.com/x", None));
        assert!(!allowed("https://api.github.com.evil.com/x", None));
        assert!(allowed(
            "http://localhost:8080/search?q=x",
            Some("http://localhost:8080")
        ));
        assert!(!allowed(
            "http://searx.example.org/search",
            Some("http://searx.example.org")
        ));
        assert!(allowed(
            "https://searx.example.org/search",
            Some("https://searx.example.org")
        ));
        // Pack sources: only the rosdistro index and GitHub source archives.
        assert!(allowed(
            "https://raw.githubusercontent.com/ros/rosdistro/master/jazzy/distribution.yaml",
            None
        ));
        assert!(!allowed(
            "https://raw.githubusercontent.com/evil/repo/main/x.yaml",
            None
        ));
        assert!(!allowed(
            "https://evil.com/ros/rosdistro/raw.githubusercontent.com",
            None
        ));
        assert!(allowed(
            "https://codeload.github.com/ros-navigation/navigation2/tar.gz/jazzy",
            None
        ));
        assert!(!allowed("https://codeload.github.com/a/b/zip/main", None));
        assert!(!allowed("http://codeload.github.com/a/b/tar.gz/main", None));
    }

    #[test]
    fn queries_contain_no_workspace_data() {
        for r in crate::rules::RULES {
            if let Some(q) = query_for_rule(r.id) {
                assert!(q.split_whitespace().count() >= 3, "{q}");
            }
        }
    }
}
