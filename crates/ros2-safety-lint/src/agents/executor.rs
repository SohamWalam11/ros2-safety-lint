//! Executor agent: callbacks that block on work only their own executor can do.

use super::{Agent, Context};
use crate::diag::Diagnostic;
use crate::facts::*;
use crate::source::{FileId, Loc};
use std::collections::HashSet;

pub struct ExecutorAgent;

impl Agent for ExecutorAgent {
    fn id(&self) -> &'static str {
        "executor"
    }
    fn title(&self) -> &'static str {
        "Executor deadlocks and blocking callbacks"
    }
    fn inputs(&self) -> &'static [crate::workspace::FileKind] {
        use crate::workspace::FileKind as K;
        // Node code, and the package files that say which executor a package uses.
        &[
            K::Cpp,
            K::Python,
            K::Manifest,
            K::CMakeLists,
            K::SetupPy,
            K::SetupCfg,
        ]
    }
    fn relevant(&self, cx: &Context) -> bool {
        cx.ix.code.values().any(|c| !c.blocking.is_empty())
    }
    fn run(&self, cx: &Context, out: &mut Vec<Diagnostic>) {
        let mut files: Vec<&FileId> = cx.ix.code.keys().collect();
        files.sort();
        for &fid in files {
            let rel = &cx.ix.file(fid).rel_path;
            if cx.config.is_test_path(rel) {
                continue;
            }
            let facts = &cx.ix.code[&fid];
            for imp in &facts.nodes {
                check_node(cx, fid, imp, out);
            }
        }
    }
}

/// Executor kind used for the package (MultiThreaded wins if any is created).
fn package_executor(cx: &Context, file: FileId) -> ExecutorKind {
    let pkg = cx.ix.file_pkg.get(file).copied().flatten();
    let mut kinds = HashSet::new();
    for (f, c) in &cx.ix.code {
        if cx.ix.file_pkg.get(*f).copied().flatten() == pkg && pkg.is_some() || *f == file {
            for e in &c.executors {
                kinds.insert(e.kind);
            }
        }
    }
    if kinds.contains(&ExecutorKind::MultiThreaded) {
        ExecutorKind::MultiThreaded
    } else if kinds.contains(&ExecutorKind::SingleThreaded) {
        ExecutorKind::SingleThreaded
    } else {
        ExecutorKind::Unknown
    }
}

/// Body ranges (file, start, end) executed by a callback, following member calls one level deep.
fn callback_ranges(
    cx: &Context,
    file: FileId,
    imp: &NodeImpl,
    cb: &Callback,
) -> Vec<(FileId, usize, usize, String)> {
    let pkg = cx.ix.file_pkg.get(file).copied().flatten();
    let find_fn = |name: &str| -> Vec<(FileId, usize, usize)> {
        let mut v = Vec::new();
        for (f, c) in &cx.ix.code {
            if *f != file && (pkg.is_none() || cx.ix.file_pkg.get(*f).copied().flatten() != pkg) {
                continue;
            }
            for fi in &c.functions {
                if fi.name == name
                    && (imp.class_name.is_none()
                        || fi.class == imp.class_name
                        || fi.class.is_none() && *f == file)
                {
                    v.push((*f, fi.start, fi.end));
                }
            }
        }
        v
    };
    let mut first: Vec<(FileId, usize, usize, String)> = match &cb.target {
        // Messages read "<kind> callback <label> …", e.g. "timer callback (lambda) sleeps".
        CallbackTarget::Inline { start, end } => vec![(file, *start, *end, "(lambda)".into())],
        CallbackTarget::Function(name) => find_fn(name)
            .into_iter()
            .map(|(f, s, e)| (f, s, e, format!("`{name}`")))
            .collect(),
    };
    // One level of member calls (e.g. a lambda that calls `this->handle(msg)`).
    let mut more = Vec::new();
    for (f, s, e, label) in &first {
        let Some(c) = cx.ix.code.get(f) else { continue };
        // Work handed to another thread (std::thread / std::async / threading.Thread) does not
        // run on the executor: do not follow calls out of such callbacks.
        let body = &cx.ix.file(*f).text[*s..(*e).min(cx.ix.file(*f).text.len())];
        if [
            "std::thread",
            "std::async",
            ".detach()",
            "threading.Thread",
            "run_in_executor",
        ]
        .iter()
        .any(|t| body.contains(t))
        {
            continue;
        }
        let called: HashSet<&str> = c
            .call_sites
            .iter()
            .filter(|(_, p)| p >= s && p < e)
            .map(|(n, _)| n.as_str())
            .collect();
        for name in called {
            for (f2, s2, e2) in find_fn(name) {
                if !first.iter().any(|x| x.0 == f2 && x.1 == s2) {
                    more.push((f2, s2, e2, format!("{label} → `{name}`")));
                }
            }
        }
    }
    first.extend(more);
    first
}

fn check_node(cx: &Context, file: FileId, imp: &NodeImpl, out: &mut Vec<Diagnostic>) {
    if imp.callbacks.is_empty() {
        return;
    }
    let exec = package_executor(cx, file);
    let no_groups = imp.groups.is_empty();
    let reentrant: HashSet<&str> = imp
        .groups
        .iter()
        .filter(|g| g.kind == GroupKind::Reentrant)
        .map(|g| g.var.as_str())
        .collect();
    let mut seen: HashSet<Loc> = HashSet::new();
    for cb in &imp.callbacks {
        for (f, s, e, label) in callback_ranges(cx, file, imp, cb) {
            let Some(c) = cx.ix.code.get(&f) else {
                continue;
            };
            for b in c
                .blocking
                .iter()
                .filter(|b| b.loc.start >= s && b.loc.start < e)
            {
                if !seen.insert(b.loc) {
                    continue;
                }
                if let Some(d) = judge(imp, cb, b, exec, no_groups, &reentrant, &label) {
                    out.push(d.related(cb.loc, "callback registered here"));
                }
            }
        }
    }
}

fn judge(
    imp: &NodeImpl,
    cb: &Callback,
    b: &BlockingCall,
    exec: ExecutorKind,
    no_groups: bool,
    reentrant: &HashSet<&str>,
    label: &str,
) -> Option<Diagnostic> {
    let cb_kind = match cb.kind {
        CallbackKind::Subscription => "subscription callback",
        CallbackKind::Timer => "timer callback",
        CallbackKind::Service => "service callback",
        CallbackKind::Action => "action callback",
    };
    match &b.kind {
        // An action's execute callback runs as an executor task outside the callback group
        // (rclpy); on a MultiThreadedExecutor its sleep blocks no other callback.
        BlockingKind::Sleep if cb.kind == CallbackKind::Action && exec == ExecutorKind::MultiThreaded => None,
        BlockingKind::Sleep => Some(Diagnostic::new(
            "E002",
            b.loc,
            format!("{cb_kind} {label} sleeps; every other callback on this executor thread waits meanwhile"),
        )),
        BlockingKind::Spin { executor: None } => Some(
            Diagnostic::new(
                "E001",
                b.loc,
                format!("{cb_kind} {label} spins the node from inside one of its own callbacks; the node is already being spun, so this deadlocks or throws"),
            )
            .help("Send the request asynchronously and handle the response in a callback (or a separate callback group on a MultiThreadedExecutor)."),
        ),
        BlockingKind::Spin { executor: Some(_) } => {
            // Spinning a dedicated executor that owns a separate callback group is the standard safe pattern.
            if !imp.dedicated_groups.is_empty() {
                return None;
            }
            None
        }
        BlockingKind::FutureWait { client } | BlockingKind::SyncCall { client } => {
            let client_ep = client.as_ref().and_then(|c| imp.endpoints.iter().find(|e| e.var.as_deref() == Some(c.as_str())));
            let client_group = client_ep.and_then(|e| e.group.clone());
            if client_group.as_ref().is_some_and(|g| imp.dedicated_groups.contains(g)) {
                return None;
            }
            let same_group = match (&cb.group, &client_group) {
                (None, None) => client_ep.is_some() || no_groups,
                (Some(a), Some(b)) => a == b && !reentrant.contains(a.as_str()),
                _ => false,
            };
            let what = if matches!(b.kind, BlockingKind::SyncCall { .. }) { "a synchronous service call" } else { "a blocking wait on a request future" };
            let client_txt = client.as_deref().map(|c| format!(" (client `{c}`)")).unwrap_or_default();
            if same_group {
                return Some(
                    Diagnostic::new(
                        "E001",
                        b.loc,
                        format!("{cb_kind} {label} makes {what}{client_txt}, but the response is delivered through the same callback group: this deadlocks"),
                    )
                    .help("Use `async_send_request` with a response callback, or create the client in a separate callback group and spin with a MultiThreadedExecutor."),
                );
            }
            if exec == ExecutorKind::SingleThreaded && client_ep.is_some() {
                return Some(
                    Diagnostic::new(
                        "E001",
                        b.loc,
                        format!("{cb_kind} {label} makes {what}{client_txt}; the client has its own callback group, but the node runs on a single-threaded executor, so the response is never processed"),
                    )
                    .help("Spin this node with a MultiThreadedExecutor, or make the call asynchronous."),
                );
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::analyze_files;

    #[test]
    fn deadlock_in_default_group() {
        let d = analyze_files(&[
            (
                "p/package.xml",
                "<package format=\"3\"><name>p</name></package>",
            ),
            (
                "p/src/n.cpp",
                r#"
class N : public rclcpp::Node {
public:
  N() : Node("n") {
    client_ = create_client<std_srvs::srv::Trigger>("trigger");
    sub_ = create_subscription<std_msgs::msg::String>("in", 10, std::bind(&N::on_msg, this, std::placeholders::_1));
    timer_ = create_wall_timer(std::chrono::seconds(1), [this]() { helper(); });
  }
  void on_msg(const std_msgs::msg::String::SharedPtr) {
    auto f = client_->async_send_request(std::make_shared<std_srvs::srv::Trigger::Request>());
    f.wait();
  }
  void helper() { rclcpp::sleep_for(std::chrono::milliseconds(5)); }
};
int main() { rclcpp::spin(std::make_shared<N>()); }
"#,
            ),
        ]);
        assert!(d.iter().any(|x| x.rule == "E001"), "{d:#?}");
        assert!(d.iter().any(|x| x.rule == "E002"), "{d:#?}");
    }

    #[test]
    fn separate_group_with_mt_executor_is_fine() {
        let d = analyze_files(&[
            (
                "p/package.xml",
                "<package format=\"3\"><name>p</name></package>",
            ),
            (
                "p/src/n.cpp",
                r#"
class N : public rclcpp::Node {
public:
  N() : Node("n") {
    cb_group_ = create_callback_group(rclcpp::CallbackGroupType::MutuallyExclusive);
    client_ = create_client<std_srvs::srv::Trigger>("trigger", rmw_qos_profile_services_default, cb_group_);
    timer_ = create_wall_timer(std::chrono::seconds(1), std::bind(&N::tick, this));
  }
  void tick() { auto f = client_->async_send_request(req_); f.get(); }
};
int main() { rclcpp::executors::MultiThreadedExecutor ex; ex.add_node(std::make_shared<N>()); ex.spin(); }
"#,
            ),
        ]);
        assert!(!d.iter().any(|x| x.rule == "E001"), "{d:#?}");
    }

    #[test]
    fn python_sync_call() {
        let d = analyze_files(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name><export><build_type>ament_python</build_type></export></package>"),
            ("p/p/n.py", "import rclpy\nfrom rclpy.node import Node\nclass N(Node):\n    def __init__(self):\n        super().__init__('n')\n        self.cli = self.create_client(T, 'srv')\n        self.t = self.create_timer(1.0, self.tick)\n    def tick(self):\n        self.cli.call(T.Request())\n"),
        ]);
        assert!(d.iter().any(|x| x.rule == "E001"), "{d:#?}");
    }

    /// rclpy runs an action's execute callback as an executor task: with a
    /// MultiThreadedExecutor its sleep blocks nothing else (demos fibonacci_action_server).
    #[test]
    fn action_sleep_depends_on_executor() {
        let server = |main: &str| {
            format!("import time\nimport rclpy\nfrom rclpy.node import Node\nfrom rclpy.action import ActionServer\nclass S(Node):\n    def __init__(self):\n        super().__init__('s')\n        self._a = ActionServer(self, Fib, 'fib', self.execute_callback)\n    def execute_callback(self, goal):\n        time.sleep(1)\n        return Fib.Result()\ndef main():\n    rclpy.init()\n    n = S()\n{main}\n")
        };
        let pkg = ("p/package.xml", "<package format=\"3\"><name>p</name><export><build_type>ament_python</build_type></export></package>");
        let mt = server("    ex = rclpy.executors.MultiThreadedExecutor()\n    rclpy.spin(n, executor=ex)");
        let d = analyze_files(&[pkg, ("p/p/s.py", &mt)]);
        assert!(d.iter().all(|x| x.rule != "E002"), "{d:#?}");
        let st = server("    rclpy.spin(n)");
        let d = analyze_files(&[pkg, ("p/p/s.py", &st)]);
        assert!(d.iter().any(|x| x.rule == "E002"), "{d:#?}");
    }
}
