use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use rosfix::config::Config;
use rosfix::engine::{self, Analysis, Options};
use rosfix::report::{self, Style};
use rosfix::rules::{Severity, RULES};
use rosfix::{fix, rules};
use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
    Sarif,
    /// Reviewdog Diagnostic Format; verified fixes become pull-request suggestions
    Rdjson,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum FailOn {
    Error,
    Warning,
    Info,
    Never,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Default)]
enum GraphFormat {
    #[default]
    Text,
    Json,
    Dot,
    Mermaid,
    /// The static callback graph (executors, callback groups, timers, subscriptions) as JSON
    Callbacks,
}

const ABOUT: &str = "Static checker for silent ROS 2 misconfigurations";
const LONG_ABOUT: &str = "Static checker for silent ROS 2 misconfigurations.

Scans a ROS 2 workspace (launch files, parameter YAML, C++/Python nodes, package
manifests, URDF/xacro, Nav2/ros2_control config, SROS2 policies), builds the launch
graph, and reports problems that otherwise only show up at runtime, or never, because
ROS 2 silently ignores them.

Without a command, `rosfix [PATH] [OPTIONS]` is the same as `rosfix check`.
Run `rosfix docs` to read the manual.";

// ------------------------------------------------------------------------------ interface

#[derive(Parser)]
#[command(
    name = "rosfix",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    override_usage = "rosfix [PATH] [OPTIONS]\n       rosfix <COMMAND> [ARGS]"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Analyse a workspace and report findings (the default command)
    Check(CheckArgs),
    /// Show verified automatic fixes as a diff; write them with --write
    Fix(FixArgs),
    /// Re-analyse on every change and print findings that appear (+) or disappear (-)
    Watch(WatchArgs),
    /// Run as a language server over stdio, for editors
    Lsp(TargetArgs),
    /// Print the resolved launch graph: nodes, and the topics that connect them
    Graph(GraphArgs),
    /// List every rule
    Rules,
    /// Explain one rule
    Explain {
        /// Rule ID, for example P003
        rule: String,
    },
    /// Record the current findings so that later runs report only new ones
    Baseline(BaselineArgs),
    /// Write rosfix.toml, a GitHub workflow and, on request, a pre-commit entry
    Init(InitArgs),
    /// Check the ROS 2 environment of this shell: middleware, domain, DDS files
    Doctor {
        /// Output format
        #[arg(long, value_enum, default_value_t = TriageFormat::Text)]
        format: TriageFormat,
    },
    /// Report what breaks when the workspace moves to another ROS distribution
    Migrate(MigrateArgs),
    /// Explain errors in a launch or node log with the workspace's findings
    Triage(TriageArgs),
    /// Compare the launch graph with a recording (rosbag2 metadata.yaml or `ros2 topic list -t`)
    CheckBag(CheckBagArgs),
    /// Write an SROS2 access-control policy for the launch graph, before anything runs
    Policy(PolicyArgs),
    /// What each agent did on this workspace, what limits it, and what to do next
    Status(StatusArgs),
    /// One HTML page with the findings, agent status and launch graph
    Report(HtmlArgs),
    /// NoDL node interface documents
    #[command(subcommand)]
    Nodl(NodlCmd),
    /// Interactive shell: run rosfix commands one after another in a workspace
    Shell {
        /// Workspace to start in
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Read the manual; without a topic, list the topics
    Docs {
        /// Topic name, or `all` for the whole manual
        topic: Option<String>,
    },
}

#[derive(Args, Clone)]
struct TargetArgs {
    /// Workspace (or package) directory to analyse
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Configuration file [default: PATH/rosfix.toml if present]
    #[arg(long)]
    config: Option<PathBuf>,
    /// ROS distribution: humble, jazzy, kilted, rolling [default: ROS_DISTRO]
    #[arg(long)]
    distro: Option<String>,
    /// Installed ROS prefix to read (repeatable) [default: AMENT_PREFIX_PATH]
    #[arg(long)]
    ros_prefix: Vec<PathBuf>,
    /// Ignore the installed ROS environment
    #[arg(long)]
    no_ros: bool,
    /// Only run these agents, comma separated (see `rosfix docs agents`)
    #[arg(long, value_delimiter = ',')]
    agents: Option<Vec<String>>,
    /// Threads for the indexing stage (needs the `parallel` feature)
    #[arg(long, default_value_t = 1)]
    jobs: usize,
}

#[derive(Args, Clone)]
struct ReportArgs {
    /// Report format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Write the report to a file instead of stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
    /// Exit with code 1 when findings at or above this severity remain [default: from config, else warning]
    #[arg(long, value_enum)]
    fail_on: Option<FailOn>,
    /// Hide findings recorded in this baseline file
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Let the fix assistant choose repairs among each finding's valid options (offline)
    #[arg(long)]
    assist: bool,
    /// Look up background references on allowlisted public sources (queries contain only the rule that fired)
    #[arg(long)]
    online: bool,
    /// With --online: print every query before it is sent
    #[arg(long)]
    show_queries: bool,
    /// Print per-agent timing and indexing statistics to stderr
    #[arg(long)]
    stats: bool,
    /// Disable colored output
    #[arg(long)]
    no_color: bool,
    /// Also print each finding's rule summary
    #[arg(long, short)]
    verbose: bool,
}

#[derive(Args)]
struct CheckArgs {
    #[command(flatten)]
    target: TargetArgs,
    #[command(flatten)]
    report: ReportArgs,
    /// Training-data tooling: print every repairable finding as JSON lines
    #[arg(long, hide = true)]
    assist_cases: bool,
}

#[derive(Args)]
struct FixArgs {
    #[command(flatten)]
    target: TargetArgs,
    #[command(flatten)]
    report: ReportArgs,
    /// Apply the verified fixes to the files on disk
    #[arg(long)]
    write: bool,
    /// Build the workspace before and after the fixes in a temporary copy and refuse fixes that break the build
    #[arg(long)]
    verify_build: bool,
    /// With --write: also write the fix assistant's choices (needs --assist)
    #[arg(long, requires = "write")]
    accept_assist: bool,
}

#[derive(Args)]
struct WatchArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// How often to check for changes, in milliseconds
    #[arg(long, default_value_t = 500)]
    interval_ms: u64,
}

#[derive(Args)]
struct GraphArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Output format
    #[arg(long, value_enum, default_value_t = GraphFormat::Text)]
    format: GraphFormat,
    /// Write the graph to a file instead of stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct BaselineArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Baseline file to write
    #[arg(long, short = 'o', default_value = "rosfix-baseline.json")]
    output: PathBuf,
}

#[derive(Args)]
struct InitArgs {
    /// Repository root to set up
    #[arg(default_value = ".")]
    path: PathBuf,
    /// ROS distribution to record in rosfix.toml [default: ROS_DISTRO]
    #[arg(long)]
    distro: Option<String>,
    /// Do not write .github/workflows/rosfix.yml
    #[arg(long)]
    no_ci: bool,
    /// Also write .pre-commit-config.yaml (an existing one is never edited)
    #[arg(long)]
    pre_commit: bool,
    /// Replace files that already exist
    #[arg(long)]
    force: bool,
    /// Print what would be written, write nothing
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct StatusArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Output format
    #[arg(long, value_enum, default_value_t = TriageFormat::Text)]
    format: TriageFormat,
}

#[derive(Args)]
struct HtmlArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Write the page to a file instead of stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct PolicyArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Only the nodes of this launch file's graph
    #[arg(long)]
    launch: Option<String>,
    /// Write the policy to a file instead of stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct CheckBagArgs {
    /// A rosbag2 directory, its metadata.yaml, or a file with `ros2 topic list -t` output
    bag: PathBuf,
    #[command(flatten)]
    target: TargetArgs,
    /// Compare only with this launch file's graph, and report topics it publishes that the recording lacks
    #[arg(long)]
    launch: Option<String>,
    /// Output format
    #[arg(long, value_enum, default_value_t = TriageFormat::Text)]
    format: TriageFormat,
    /// Write the result to a file instead of stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Subcommand)]
enum NodlCmd {
    /// Write a NoDL document for each workspace executable, from what rosfix read in its source
    Export(NodlExportArgs),
}

#[derive(Args)]
struct NodlExportArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Write `<DIR>/<package>/<executable>.nodl.yaml` files instead of printing to stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum TriageFormat {
    Text,
    Json,
}

#[derive(Args)]
struct TriageArgs {
    /// Log file from `ros2 launch` or a node; `-` reads stdin
    log: PathBuf,
    #[command(flatten)]
    target: TargetArgs,
    /// Output format
    #[arg(long, value_enum, default_value_t = TriageFormat::Text)]
    format: TriageFormat,
    /// Write the result to a file instead of stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct MigrateArgs {
    /// Workspace (or package) directory to analyse
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Target distribution: humble, jazzy, kilted, rolling
    #[arg(long)]
    to: String,
    /// Current distribution [default: `distro` in rosfix.toml, else ROS_DISTRO]
    #[arg(long)]
    from: Option<String>,
    /// Configuration file [default: PATH/rosfix.toml if present]
    #[arg(long)]
    config: Option<PathBuf>,
    /// Only run these agents, comma separated
    #[arg(long, value_delimiter = ',')]
    agents: Option<Vec<String>>,
    /// Report format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Write the report to a file instead of stdout
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
    /// Exit with code 1 when new findings at or above this severity exist [default: from config, else warning]
    #[arg(long, value_enum)]
    fail_on: Option<FailOn>,
    /// Show verified fixes for the new findings as a diff (checked against the target distribution)
    #[arg(long)]
    fix: bool,
    /// With --fix: apply them
    #[arg(long, requires = "fix")]
    write: bool,
    /// Disable colored output
    #[arg(long)]
    no_color: bool,
    /// Also print each finding's rule summary
    #[arg(long, short)]
    verbose: bool,
}

/// `rosfix [PATH] [OPTIONS]`: the original single-command interface, unchanged so that
/// scripts, the GitHub Action and the pre-commit hook keep working. Every subcommand is
/// translated into this description of one run.
#[derive(Parser, Clone)]
#[command(name = "rosfix", version, about = ABOUT)]
struct Run {
    /// Workspace (or package) directory to analyse.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Same as the positional PATH (kept for compatibility).
    #[arg(long = "path", hide = true)]
    path_flag: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Write the report to a file instead of stdout.
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,

    /// Exit with code 1 when findings at or above this severity remain (default from config: warning).
    #[arg(long, value_enum)]
    fail_on: Option<FailOn>,

    /// Compute verified automatic fixes and print them as a unified diff.
    #[arg(long)]
    fix: bool,

    /// With --fix: apply the verified fixes to the files on disk.
    #[arg(long, requires = "fix")]
    write: bool,

    /// Only run these agents (comma separated): params,topics,executor,control,tf,lifecycle,launch,network,packaging,simtime,qos,nav2,security.
    #[arg(long, value_delimiter = ',')]
    agents: Option<Vec<String>>,

    /// Threads for the indexing stage (needs the `parallel` feature; default 1).
    #[arg(long, default_value_t = 1)]
    jobs: usize,

    /// Hide findings recorded in this baseline file.
    #[arg(long)]
    baseline: Option<PathBuf>,

    /// Record all current findings into a baseline file and exit 0.
    #[arg(long)]
    write_baseline: Option<PathBuf>,

    /// Configuration file (default: <path>/rosfix.toml if present).
    #[arg(long)]
    config: Option<PathBuf>,

    /// Print per-agent timing and indexing statistics to stderr.
    #[arg(long)]
    stats: bool,

    /// Let the fix assistant choose repairs for findings without a deterministic fix
    /// (built-in learned ranker over the finding's valid options). Runs fully offline.
    #[arg(long)]
    assist: bool,

    /// Look up background references for findings on allowlisted public sources
    /// (Robotics Stack Exchange, GitHub issues, your SearXNG). Queries contain only the
    /// rule that fired — never your files, paths or names. Off unless requested.
    #[arg(long)]
    online: bool,

    /// With --fix: build the workspace before and after the fixes in a temporary copy
    /// (Docker without network if available, else local colcon); refuse to write fixes
    /// that break a build that worked.
    #[arg(long, requires = "fix")]
    verify_build: bool,

    /// With --online: print every query before it is sent.
    #[arg(long)]
    show_queries: bool,

    /// With --fix --write: also write assisted fixes (they are always verified first).
    #[arg(long, requires = "write")]
    accept_assist: bool,

    /// Installed ROS prefix to read (repeatable). Default: AMENT_PREFIX_PATH of the current shell.
    #[arg(long)]
    ros_prefix: Vec<PathBuf>,

    /// Ignore the installed ROS environment.
    #[arg(long)]
    no_ros: bool,

    /// ROS distribution (humble, jazzy, kilted, rolling). Default: ROS_DISTRO.
    #[arg(long)]
    distro: Option<String>,

    /// Training-data tooling: print every repairable finding (wrong text + options) as JSON lines.
    #[arg(long, hide = true)]
    assist_cases: bool,

    /// Keep running: re-analyse incrementally whenever a file changes and print the
    /// findings that appear (+) or disappear (-).
    #[arg(long, conflicts_with_all = ["fix", "lsp"])]
    watch: bool,

    /// With --watch: how often to check for changes, in milliseconds.
    #[arg(long, default_value_t = 500)]
    watch_interval_ms: u64,

    /// Run as a Language Server over stdio (diagnostics for editors, incl. unsaved buffers).
    #[arg(long, conflicts_with = "fix")]
    lsp: bool,

    /// Print the resolved launch graph (nodes, namespaces, linked sources) and exit.
    #[arg(long)]
    print_graph: bool,

    /// List all rules and exit.
    #[arg(long)]
    list_rules: bool,

    /// Explain one rule (e.g. `--explain P003`) and exit.
    #[arg(long)]
    explain: Option<String>,

    /// Disable colored output.
    #[arg(long)]
    no_color: bool,

    #[arg(long, short)]
    verbose: bool,

    /// Set by `rosfix graph --format`.
    #[arg(skip)]
    graph_format: GraphFormat,
}

impl Run {
    fn new(t: &TargetArgs) -> Run {
        let mut r = Run::parse_from(["rosfix"]);
        r.path = t.path.clone();
        r.config = t.config.clone();
        r.distro = t.distro.clone();
        r.ros_prefix = t.ros_prefix.clone();
        r.no_ros = t.no_ros;
        r.agents = t.agents.clone();
        r.jobs = t.jobs;
        r
    }

    fn report(mut self, p: &ReportArgs) -> Run {
        self.format = p.format;
        self.output = p.output.clone();
        self.fail_on = p.fail_on;
        self.baseline = p.baseline.clone();
        self.assist = p.assist;
        self.online = p.online;
        self.show_queries = p.show_queries;
        self.stats = p.stats;
        self.no_color = p.no_color;
        self.verbose = p.verbose;
        self
    }
}

// ------------------------------------------------------------------------------ entry

/// Which interface the arguments use: a subcommand (or a request for help/version), or the
/// original flags. `rosfix ./check` analyses a directory named `check`.
fn uses_commands(args: &[OsString]) -> bool {
    let Some(first) = args.get(1).and_then(|a| a.to_str()) else {
        return false;
    };
    first == "help"
        || Cli::command().find_subcommand(first).is_some()
        || matches!(first, "-h" | "--help" | "-V" | "--version")
}

fn main() -> ExitCode {
    // Internal: the sandboxed network helper used by --online (never invoked by users).
    if std::env::args().nth(1).as_deref() == Some("__fetch") {
        return ExitCode::from(rosfix::assist::online::helper_main() as u8);
    }
    let args: Vec<OsString> = std::env::args_os().collect();
    let result = if uses_commands(&args) {
        dispatch(Cli::parse_from(&args))
    } else if args.iter().skip(1).any(|a| a == "-h" || a == "--help") {
        // `rosfix . --help`: show the command overview, not the long flag list.
        Cli::parse_from(["rosfix", "--help"]);
        unreachable!("--help exits")
    } else {
        run(Run::parse_from(&args))
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rosfix: error: {e}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode, String> {
    match cli.cmd {
        Cmd::Check(a) => {
            let mut r = Run::new(&a.target).report(&a.report);
            r.assist_cases = a.assist_cases;
            run(r)
        }
        Cmd::Fix(a) => {
            let mut r = Run::new(&a.target).report(&a.report);
            r.fix = true;
            r.write = a.write;
            r.verify_build = a.verify_build;
            r.accept_assist = a.accept_assist;
            run(r)
        }
        Cmd::Watch(a) => {
            let mut r = Run::new(&a.target);
            r.watch = true;
            r.watch_interval_ms = a.interval_ms;
            run(r)
        }
        Cmd::Lsp(t) => {
            let mut r = Run::new(&t);
            r.lsp = true;
            run(r)
        }
        Cmd::Graph(a) => {
            let mut r = Run::new(&a.target);
            r.print_graph = true;
            r.graph_format = a.format;
            r.output = a.output;
            run(r)
        }
        Cmd::Rules => {
            let mut r = Run::parse_from(["rosfix"]);
            r.list_rules = true;
            run(r)
        }
        Cmd::Explain { rule } => {
            let mut r = Run::parse_from(["rosfix"]);
            r.explain = Some(rule);
            run(r)
        }
        Cmd::Baseline(a) => {
            let mut r = Run::new(&a.target);
            r.write_baseline = Some(a.output);
            run(r)
        }
        Cmd::Init(a) => init(a),
        Cmd::Doctor { format } => doctor(format),
        Cmd::Migrate(a) => migrate(a),
        Cmd::Triage(a) => triage(a),
        Cmd::Nodl(NodlCmd::Export(a)) => nodl_export(a),
        Cmd::CheckBag(a) => check_bag(a),
        Cmd::Policy(a) => policy(a),
        Cmd::Status(a) => status(a),
        Cmd::Report(a) => html_report(a),
        Cmd::Shell { path } => shell(&path),
        Cmd::Docs { topic } => {
            print!("{}", rosfix::docs::render(topic.as_deref())?);
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ------------------------------------------------------------------------------ shared steps

fn load_config(root: &Path, explicit: Option<&PathBuf>) -> Result<Config, String> {
    match explicit {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let raw = toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            Config::from_raw(raw, Some(p.display().to_string()))
        }
        None => Config::load_from_dir(if root.is_dir() {
            root
        } else {
            root.parent().unwrap_or(root)
        }),
    }
}

fn check_agents(agents: &Option<Vec<String>>) -> Result<(), String> {
    let Some(a) = agents else { return Ok(()) };
    let known: Vec<&str> = rosfix::agents::all_agents()
        .iter()
        .map(|x| x.id())
        .collect();
    match a.iter().find(|x| !known.contains(&x.as_str())) {
        Some(bad) => Err(format!(
            "unknown agent `{bad}` (known: {})",
            known.join(", ")
        )),
        None => Ok(()),
    }
}

fn existing(root: &Path) -> Result<(), String> {
    if root.exists() {
        Ok(())
    } else {
        Err(format!("path `{}` does not exist", root.display()))
    }
}

fn env_distro() -> Option<String> {
    std::env::var("ROS_DISTRO").ok().filter(|d| !d.is_empty())
}

/// Writes a report to a file, or to stdout.
fn emit(body: &str, output: Option<&PathBuf>) -> Result<(), String> {
    match output {
        Some(p) => std::fs::write(p, body).map_err(|e| format!("{}: {e}", p.display())),
        None => {
            let _ = std::io::stdout().lock().write_all(body.as_bytes());
            Ok(())
        }
    }
}

/// `verified`: the fixes that passed verification, for formats that carry fixes (rdjson).
fn render(
    a: &Analysis,
    format: Format,
    color: bool,
    verbose: bool,
    verified: Option<&rosfix::fix::FixReport>,
) -> String {
    match format {
        Format::Text => report::text(a, &Style { color }, verbose),
        Format::Json => report::json(a),
        Format::Sarif => report::sarif(a),
        Format::Rdjson => report::rdjson(a, verified),
    }
}

fn stdout_color(no_color: bool, output: Option<&PathBuf>) -> bool {
    !no_color
        && output.is_none()
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
}

fn stderr_style(no_color: bool) -> Style {
    Style {
        color: !no_color
            && std::io::stderr().is_terminal()
            && std::env::var_os("NO_COLOR").is_none(),
    }
}

fn threshold(fail_on: Option<FailOn>, config: &Config) -> Option<Severity> {
    match fail_on {
        Some(FailOn::Error) => Some(Severity::Error),
        Some(FailOn::Warning) => Some(Severity::Warning),
        Some(FailOn::Info) => Some(Severity::Info),
        Some(FailOn::Never) => None,
        None => Some(config.fail_on),
    }
}

/// Exit code 1 when a finding at or above the threshold remains. Findings fixed on disk
/// by `--write` no longer count.
fn exit_code(
    a: &Analysis,
    threshold: Option<Severity>,
    fixed: &[(&'static str, String)],
) -> ExitCode {
    let failing = threshold.is_some_and(|t| {
        a.diagnostics
            .iter()
            .any(|d| d.severity >= t && !fixed.iter().any(|(r, m)| *r == d.rule && *m == d.message))
    });
    if failing {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

struct FixRequest {
    write: bool,
    verify_build: bool,
    include_assisted: bool,
    /// The diff goes to stderr when a machine-readable report owns stdout.
    diff_to_stdout: bool,
}

/// Verifies the fixes for `a`, then shows them as a diff or writes them. Returns the
/// findings that were fixed on disk.
fn apply_fixes(
    root: &Path,
    config: &Config,
    opts: &Options,
    a: &Analysis,
    req: &FixRequest,
) -> Result<Vec<(&'static str, String)>, String> {
    let r = fix::run(root, config, opts, a, req.include_assisted);
    let diff = fix::unified_diff(&r);
    if req.verify_build && !diff.is_empty() {
        eprintln!("rosfix: building the workspace before and after the fixes (temporary copies)…");
        match rosfix::sandbox::verify_build(root, &r, opts.distro.as_deref()) {
            Ok(rosfix::sandbox::Outcome::Passed) => eprintln!("rosfix: build check passed"),
            Ok(rosfix::sandbox::Outcome::Inconclusive(log)) => {
                eprintln!("rosfix: build check inconclusive: the unmodified workspace does not build either\n{log}")
            }
            Ok(rosfix::sandbox::Outcome::Failed(log)) => {
                return Err(format!(
                    "the fixes break a build that worked; nothing was written\n{log}"
                ));
            }
            Err(e) => return Err(format!("--verify-build: {e}")),
        }
    }
    let mut fixed = Vec::new();
    if diff.is_empty() {
        eprintln!("rosfix: no automatic fixes available");
    } else if req.write {
        let n = fix::write(&r).map_err(|e| format!("writing fixes: {e}"))?;
        eprintln!(
            "rosfix: applied {} verified fix(es) to {n} file(s)",
            r.applied.len()
        );
        fixed = r
            .applied
            .iter()
            .map(|f| (f.rule, f.message.clone()))
            .collect();
    } else {
        if req.diff_to_stdout {
            print!("{diff}");
        } else {
            eprint!("{diff}");
        }
        eprintln!(
            "rosfix: {} verified fix(es) shown above; run `rosfix fix --write` to apply",
            r.applied.len()
        );
    }
    for (f, why) in &r.rejected {
        eprintln!(
            "rosfix: skipped fix for {} ({}): {why}",
            f.rule, f.description
        );
    }
    Ok(fixed)
}

/// The workspace root, its configuration and the analysis options of one run.
fn prepare(cli: &Run) -> Result<(PathBuf, Config, Options), String> {
    let root = cli.path_flag.clone().unwrap_or(cli.path.clone());
    existing(&root)?;
    let config = load_config(&root, cli.config.as_ref())?;
    if cli.jobs > 1 && !cfg!(feature = "parallel") {
        eprintln!("rosfix: note: --jobs needs the `parallel` feature; running single-threaded");
    }
    check_agents(&cli.agents)?;
    // Installed ROS: explicit --ros-prefix, else the sourced environment (AMENT_PREFIX_PATH).
    let prefixes = if cli.no_ros {
        Vec::new()
    } else if !cli.ros_prefix.is_empty() {
        cli.ros_prefix.clone()
    } else {
        rosfix::installed::prefixes_from_env()
    };
    let distro = cli
        .distro
        .clone()
        .or_else(|| config.distro.clone())
        .or_else(env_distro);
    let opts = Options {
        jobs: cli.jobs,
        agents: cli.agents.clone(),
        overrides: Default::default(),
        prefixes,
        distro,
        // Fix verification re-analyses the workspace; keep parsed facts so that only the
        // patched files are parsed again. (Not kept otherwise, to keep memory low.)
        cache: (cli.fix || cli.format == Format::Rdjson).then(Default::default),
        previous: None,
    };
    Ok((root, config, opts))
}

// ------------------------------------------------------------------------------ commands

/// Splits a shell line into words: whitespace separates, '…' and "…" group, \ escapes the
/// next character outside single quotes. Returns None for an unterminated quote.
fn split_words(line: &str) -> Option<Vec<String>> {
    let (mut out, mut cur, mut quote, mut in_word) =
        (Vec::new(), String::new(), None::<char>, false);
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some('"'), '\\') => cur.push(chars.next()?),
            (Some(_), _) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                cur.push(chars.next()?);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if in_word {
        out.push(cur);
    }
    Some(out)
}

const SHELL_HELP: &str = "Type a rosfix command without the `rosfix` prefix, for example:

    status                     what each agent checked and what limits it
    check                      findings (check --format json, check -v, ...)
    fix                        verified fixes as a diff; fix --write applies them
    graph --format mermaid     the launch graph
    explain P003               one rule; `rules` lists them all
    docs                       the manual; docs TOPIC for one topic

Every command takes --help. Commands run on the current workspace unless you give a path.
Shell commands:

    cd PATH        change the workspace        pwd       show it
    history        commands typed so far       help      this text
    exit, quit     leave (also Ctrl-D, or Ctrl-Z then Enter on Windows)

Keys: Up and Down walk the history (kept in ~/.rosfix_history), Ctrl-R searches it, Tab
completes commands, rule ids, docs topics, --format values and paths, Ctrl-C clears the
line.
";

/// Tab completion for the shell: commands, then rule ids / docs topics / formats / paths.
struct ShellHelper {
    commands: Vec<String>,
    files: rustyline::completion::FilenameCompleter,
}

impl rustyline::completion::Completer for ShellHelper {
    type Candidate = rustyline::completion::Pair;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let before = &line[..pos];
        let start = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        let word = &before[start..];
        let words: Vec<&str> = before[..start]
            .split_whitespace()
            .filter(|w| *w != "rosfix")
            .collect();
        let pick = |options: &mut dyn Iterator<Item = String>| {
            let mut v: Vec<rustyline::completion::Pair> = options
                .filter(|o| o.starts_with(word))
                .map(|o| rustyline::completion::Pair {
                    display: o.clone(),
                    replacement: o,
                })
                .collect();
            v.sort_by(|a, b| a.display.cmp(&b.display));
            v.dedup_by(|a, b| a.display == b.display);
            Ok((start, v))
        };
        match (words.first().copied(), words.last().copied()) {
            (None, _) => pick(&mut self.commands.iter().cloned()),
            (_, Some("--format")) => pick(
                &mut [
                    "text",
                    "json",
                    "sarif",
                    "rdjson",
                    "dot",
                    "mermaid",
                    "callbacks",
                ]
                .iter()
                .map(|s| s.to_string()),
            ),
            (Some("explain"), _) => {
                pick(&mut rosfix::rules::RULES.iter().map(|r| r.id.to_string()))
            }
            (Some("docs"), _) => pick(&mut rosfix::docs::topic_names().map(str::to_string)),
            _ => self.files.complete(line, pos, ctx),
        }
    }
}

impl rustyline::hint::Hinter for ShellHelper {
    type Hint = String;
}
impl rustyline::highlight::Highlighter for ShellHelper {}
impl rustyline::validate::Validator for ShellHelper {}
impl rustyline::Helper for ShellHelper {}

/// Where the shell reads its lines: an editor with history and completion on a terminal,
/// plain lines otherwise (pipes, scripts, tests).
enum ShellInput {
    Editor(
        Box<rustyline::Editor<ShellHelper, rustyline::history::DefaultHistory>>,
        Option<PathBuf>,
    ),
    Plain(std::io::Lines<std::io::StdinLock<'static>>),
}

impl ShellInput {
    fn open() -> ShellInput {
        use std::io::BufRead;
        if std::io::stdin().is_terminal() {
            if let Ok(mut ed) =
                rustyline::Editor::<ShellHelper, rustyline::history::DefaultHistory>::new()
            {
                let commands = <Cli as clap::CommandFactory>::command()
                    .get_subcommands()
                    .map(|c| c.get_name().to_string())
                    .chain(["cd", "pwd", "history", "help", "exit", "quit"].map(String::from))
                    .collect();
                ed.set_helper(Some(ShellHelper {
                    commands,
                    files: rustyline::completion::FilenameCompleter::new(),
                }));
                let file = std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .map(|h| PathBuf::from(h).join(".rosfix_history"));
                if let Some(f) = &file {
                    let _ = ed.load_history(f);
                }
                return ShellInput::Editor(Box::new(ed), file);
            }
        }
        ShellInput::Plain(std::io::stdin().lock().lines())
    }

    /// The next line, or None at end of input.
    fn read(&mut self, prompt: &str) -> Option<String> {
        match self {
            ShellInput::Editor(ed, _) => loop {
                match ed.readline(prompt) {
                    Ok(l) => {
                        if !l.trim().is_empty() {
                            let _ = ed.add_history_entry(l.as_str());
                        }
                        return Some(l);
                    }
                    // Ctrl-C clears the line instead of leaving the shell.
                    Err(rustyline::error::ReadlineError::Interrupted) => continue,
                    Err(_) => return None,
                }
            },
            ShellInput::Plain(lines) => {
                print!("{prompt}");
                let _ = std::io::stdout().flush();
                lines.next().and_then(Result::ok)
            }
        }
    }
}

impl Drop for ShellInput {
    fn drop(&mut self) {
        if let ShellInput::Editor(ed, Some(f)) = self {
            let _ = ed.save_history(f);
        }
    }
}

/// `rosfix shell`: a prompt that runs rosfix commands in a workspace until `exit`.
fn shell(path: &Path) -> Result<ExitCode, String> {
    existing(path)?;
    std::env::set_current_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let here = || {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into())
    };
    println!(
        "rosfix {} shell, workspace {}",
        env!("CARGO_PKG_VERSION"),
        here()
    );
    println!("Type `help` for the commands, `exit` to leave.\n");
    let mut input = ShellInput::open();
    let mut history: Vec<String> = Vec::new();
    loop {
        let name = std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let Some(line) = input.read(&format!("rosfix:{name}> ")) else {
            println!();
            break;
        };
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        history.push(line.to_string());
        let Some(mut words) = split_words(line) else {
            eprintln!("rosfix: unterminated quote");
            continue;
        };
        if words.first().is_some_and(|w| w == "rosfix") {
            words.remove(0);
        }
        let Some(first) = words.first().cloned() else {
            continue;
        };
        match first.as_str() {
            "exit" | "quit" => break,
            "help" | "?" if words.len() == 1 => {
                print!("{SHELL_HELP}");
                continue;
            }
            "pwd" => {
                println!("{}", here());
                continue;
            }
            "cd" => {
                match words.get(1) {
                    Some(d) => match std::env::set_current_dir(d) {
                        Ok(()) => println!("workspace {}", here()),
                        Err(e) => eprintln!("rosfix: cd {d}: {e}"),
                    },
                    None => eprintln!("rosfix: cd needs a directory"),
                }
                continue;
            }
            "history" => {
                for (i, h) in history.iter().enumerate() {
                    println!("{:>4}  {h}", i + 1);
                }
                continue;
            }
            "shell" => {
                eprintln!("rosfix: already in the shell");
                continue;
            }
            "lsp" => {
                eprintln!("rosfix: the language server talks to an editor over stdin; start it as `rosfix lsp` from the editor");
                continue;
            }
            "watch" => eprintln!("rosfix: watch runs until Ctrl-C, which also ends the shell"),
            _ => {}
        }
        let mut args: Vec<OsString> = vec!["rosfix".into()];
        args.extend(words.iter().map(OsString::from));
        let result = if uses_commands(&args) {
            match Cli::try_parse_from(&args) {
                Ok(cli) => dispatch(cli),
                Err(e) => {
                    let _ = e.print();
                    continue;
                }
            }
        } else {
            // Old-style flags (`--fix --write`, `--print-graph`, ...) work here too.
            match Run::try_parse_from(&args) {
                Ok(run_args) => run(run_args),
                Err(e) => {
                    let _ = e.print();
                    continue;
                }
            }
        };
        match result {
            Ok(code) if code == ExitCode::SUCCESS => {}
            Ok(_) => println!(
                "(exit code 1: findings at or above the --fail-on level, or a check that failed)"
            ),
            Err(e) => eprintln!("rosfix: error: {e}"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn status(a: StatusArgs) -> Result<ExitCode, String> {
    let (root, config, opts) = prepare(&Run::new(&a.target))?;
    let analysis = engine::analyze(&root, &config, &opts);
    let st = rosfix::status::status(&analysis);
    match a.format {
        TriageFormat::Text => print!("{}", rosfix::status::text(&st)),
        TriageFormat::Json => println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default()),
    }
    Ok(ExitCode::SUCCESS)
}

fn html_report(a: HtmlArgs) -> Result<ExitCode, String> {
    let (root, config, opts) = prepare(&Run::new(&a.target))?;
    let analysis = engine::analyze(&root, &config, &opts);
    let st = rosfix::status::status(&analysis);
    emit(&rosfix::html::page(&analysis, &st), a.output.as_ref())?;
    if let Some(p) = &a.output {
        eprintln!("rosfix: wrote {}", p.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn policy(a: PolicyArgs) -> Result<ExitCode, String> {
    let (root, config, opts) = prepare(&Run::new(&a.target))?;
    let analysis = engine::analyze(&root, &config, &opts);
    let (xml, incomplete) = rosfix::policy::generate(&analysis, a.launch.as_deref())?;
    emit(&xml, a.output.as_ref())?;
    if incomplete > 0 {
        eprintln!("rosfix: {incomplete} profile(s) are incomplete (marked in the policy); complete them with `ros2 security generate_policy`");
    }
    Ok(ExitCode::SUCCESS)
}

fn check_bag(a: CheckBagArgs) -> Result<ExitCode, String> {
    let file = if a.bag.is_dir() {
        a.bag.join("metadata.yaml")
    } else {
        a.bag.clone()
    };
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let observed =
        rosfix::live::read_observed(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let (root, config, opts) = prepare(&Run::new(&a.target))?;
    let analysis = engine::analyze(&root, &config, &opts);
    let items = rosfix::live::compare(&analysis, &observed, a.launch.as_deref())?;
    let body = match a.format {
        TriageFormat::Text => rosfix::live::text(&items, observed.len()),
        TriageFormat::Json => {
            serde_json::to_string_pretty(&serde_json::json!({"recorded": observed, "items": items}))
                .unwrap_or_default()
                + "\n"
        }
    };
    emit(&body, a.output.as_ref())?;
    // A type mismatch is a definite inconsistency; the other kinds may be explained by
    // what was recorded or by publishers outside the workspace.
    Ok(
        if items
            .iter()
            .any(|i| i.kind == rosfix::live::Kind::TypeMismatch)
        {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        },
    )
}

fn nodl_export(a: NodlExportArgs) -> Result<ExitCode, String> {
    let (root, config, opts) = prepare(&Run::new(&a.target))?;
    let analysis = engine::analyze(&root, &config, &opts);
    let docs = rosfix::nodl::export(&analysis);
    match &a.output {
        None => {
            for d in &docs {
                println!("# {}/{}", d.package, d.executable);
                print!("{}", d.text);
            }
        }
        Some(dir) => {
            for d in &docs {
                let p = dir
                    .join(&d.package)
                    .join(format!("{}.nodl.yaml", d.executable));
                std::fs::create_dir_all(p.parent().unwrap_or(dir))
                    .map_err(|e| format!("{}: {e}", p.display()))?;
                std::fs::write(&p, &d.text).map_err(|e| format!("{}: {e}", p.display()))?;
            }
            eprintln!(
                "rosfix: wrote {} NoDL document(s) to {}",
                docs.len(),
                dir.display()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn triage(a: TriageArgs) -> Result<ExitCode, String> {
    let log = if a.log.as_os_str() == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
            .map_err(|e| format!("stdin: {e}"))?;
        s
    } else {
        let bytes = std::fs::read(&a.log).map_err(|e| format!("{}: {e}", a.log.display()))?;
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let (root, config, opts) = prepare(&Run::new(&a.target))?;
    let analysis = engine::analyze(&root, &config, &opts);
    let items = rosfix::triage::run(&log, &analysis);
    let body = match a.format {
        TriageFormat::Text => rosfix::triage::render(&items),
        TriageFormat::Json => serde_json::to_string_pretty(&items).unwrap_or_default() + "\n",
    };
    emit(&body, a.output.as_ref())?;
    Ok(ExitCode::SUCCESS)
}

fn run(cli: Run) -> Result<ExitCode, String> {
    if cli.list_rules {
        for r in RULES {
            println!(
                "{}  {:<9} {:<7} {:<36} {}",
                r.id,
                r.agent,
                r.severity.as_str(),
                r.name,
                if r.enabled_by_default {
                    ""
                } else {
                    "(off by default)"
                }
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(id) = &cli.explain {
        let r =
            rules::find(id).ok_or_else(|| format!("unknown rule `{id}` (see `rosfix rules`)"))?;
        println!(
            "{} ({}) — agent: {}, default severity: {}\n\n{}\n\n{}",
            r.id,
            r.name,
            r.agent,
            r.severity.as_str(),
            r.summary,
            r.explanation
        );
        return Ok(ExitCode::SUCCESS);
    }
    let (root, config, opts) = prepare(&cli)?;
    if cli.watch {
        rosfix::session::watch(&root, config, opts, cli.watch_interval_ms.max(50));
    }
    if cli.lsp {
        let stdin = std::io::stdin();
        rosfix::session::lsp(
            &root,
            config,
            opts,
            &mut stdin.lock(),
            &mut std::io::stdout(),
        );
        return Ok(ExitCode::SUCCESS);
    }
    let mut analysis = engine::analyze(&root, &config, &opts);
    if cli.assist_cases {
        // Training-data tooling: one JSON line per finding that has a repair site.
        for d in &analysis.diagnostics {
            let Some(r) = &d.repair else { continue };
            let top: Vec<String> = rosfix::assist::lexical::rank(&r.current, &r.candidates)
                .into_iter()
                .take(rosfix::assist::TOP_K)
                .map(|x| x.0)
                .collect();
            let f = analysis.index.file(r.span.file);
            let line = serde_json::json!({
                "rule": d.rule,
                "file": f.rel_path,
                "line": f.line_col(r.span.start).0,
                "current": r.current,
                "options": top,
            });
            println!("{line}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if cli.print_graph {
        let body = match cli.graph_format {
            GraphFormat::Text => report::graph(&analysis),
            GraphFormat::Json => report::graph_json(&analysis),
            GraphFormat::Dot => report::graph_dot(&analysis),
            GraphFormat::Mermaid => report::graph_mermaid(&analysis),
            GraphFormat::Callbacks => report::graph_callbacks(&analysis),
        };
        emit(&body, cli.output.as_ref())?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(p) = &cli.write_baseline {
        let n =
            engine::write_baseline(p, &analysis).map_err(|e| format!("{}: {e}", p.display()))?;
        eprintln!("rosfix: wrote {n} fingerprint(s) to {}", p.display());
        return Ok(ExitCode::SUCCESS);
    }
    let hidden = match &cli.baseline {
        Some(p) => engine::apply_baseline(p, &mut analysis)?,
        None => 0,
    };

    // Fix assistant: pick repairs from each finding's closed candidate set.
    if cli.assist {
        let model = rosfix::assist::load_model();
        let proposals = rosfix::assist::propose(&analysis, Some(model.as_ref()));
        rosfix::assist::attach(&mut analysis, proposals);
    }
    if (cli.online || config.online) && !analysis.diagnostics.is_empty() {
        if !cfg!(feature = "online") {
            eprintln!("rosfix: note: this build has no online support; skipping --online");
        } else if let Err(e) = rosfix::assist::online::lookup(
            &mut analysis,
            config.searxng_url.clone(),
            cli.show_queries,
        ) {
            eprintln!("rosfix: online lookup failed: {e}");
        }
    }

    let color = stdout_color(cli.no_color, cli.output.as_ref());
    // Suggestions in rdjson are only fixes that pass the same verification as `fix`.
    let verified = (cli.format == Format::Rdjson)
        .then(|| rosfix::fix::run(&root, &config, &opts, &analysis, false));
    emit(
        &render(&analysis, cli.format, color, cli.verbose, verified.as_ref()),
        cli.output.as_ref(),
    )?;
    if cli.format == Format::Text || cli.output.is_some() {
        eprintln!(
            "{}",
            report::summary(&analysis, &stderr_style(cli.no_color), hidden)
        );
    }
    if cli.stats {
        eprint!("{}", report::agents_table(&analysis));
    }

    let mut fixed = Vec::new();
    if cli.fix {
        let req = FixRequest {
            write: cli.write,
            verify_build: cli.verify_build,
            // Assisted fixes are previewed with --assist, but written only with --accept-assist.
            include_assisted: cli.assist && (!cli.write || cli.accept_assist),
            diff_to_stdout: cli.format == Format::Text && cli.output.is_none(),
        };
        fixed = apply_fixes(&root, &config, &opts, &analysis, &req)?;
    }
    Ok(exit_code(
        &analysis,
        threshold(cli.fail_on, &config),
        &fixed,
    ))
}

fn migrate(a: MigrateArgs) -> Result<ExitCode, String> {
    existing(&a.path)?;
    let config = load_config(&a.path, a.config.as_ref())?;
    check_agents(&a.agents)?;
    let from = a.from.clone().or_else(|| config.distro.clone()).or_else(env_distro).ok_or(
        "the current distribution is unknown: pass --from, set `distro` in rosfix.toml, or source a ROS environment",
    )?;
    let base = Options {
        agents: a.agents.clone(),
        cache: a.fix.then(Default::default),
        ..Default::default()
    };
    let m = rosfix::migrate::run(&a.path, &config, &base, &from, &a.to)?;
    let color = stdout_color(a.no_color, a.output.as_ref());
    emit(
        &render(&m.after, a.format, color, a.verbose, None),
        a.output.as_ref(),
    )?;
    eprintln!(
        "rosfix: {} -> {}: {} new finding(s); {} also reported on {} (not shown); {} no longer reported on {}",
        m.from,
        m.to,
        m.after.diagnostics.len(),
        m.unchanged,
        m.from,
        m.resolved,
        m.to
    );
    let mut fixed = Vec::new();
    if a.fix {
        let req = FixRequest {
            write: a.write,
            verify_build: false,
            include_assisted: false,
            diff_to_stdout: a.format == Format::Text && a.output.is_none(),
        };
        fixed = apply_fixes(&a.path, &config, &m.opts, &m.after, &req)?;
    }
    Ok(exit_code(&m.after, threshold(a.fail_on, &config), &fixed))
}

fn doctor(format: TriageFormat) -> Result<ExitCode, String> {
    let env: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    let checks = rosfix::doctor::run(&env);
    match format {
        TriageFormat::Text => print!("{}", rosfix::doctor::text(&checks)),
        TriageFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&checks).unwrap_or_default()
        ),
    }
    Ok(
        if checks
            .iter()
            .any(|c| c.level == rosfix::doctor::Level::Error)
        {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        },
    )
}

fn init(a: InitArgs) -> Result<ExitCode, String> {
    existing(&a.path)?;
    let o = rosfix::init::InitOptions {
        distro: a.distro.clone().or_else(env_distro),
        ci: !a.no_ci,
        pre_commit: a.pre_commit,
        force: a.force,
        dry_run: a.dry_run,
    };
    let outcomes = rosfix::init::run(&a.path, &o).map_err(|e| e.to_string())?;
    for x in &outcomes {
        let path = x.path.strip_prefix(&a.path).unwrap_or(&x.path).display();
        let verb = if a.dry_run { "would write" } else { "" };
        match &x.action {
            rosfix::init::Action::Created => {
                println!("{:<9} {path}", if a.dry_run { verb } else { "created" })
            }
            rosfix::init::Action::Replaced => {
                println!("{:<9} {path}", if a.dry_run { verb } else { "replaced" })
            }
            rosfix::init::Action::Kept(why) => println!("{:<9} {path}: {why}", "kept"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::split_words;

    #[test]
    fn shell_words() {
        assert_eq!(
            split_words("check src --format json").unwrap(),
            ["check", "src", "--format", "json"]
        );
        assert_eq!(
            split_words("cd 'My Workspace'").unwrap(),
            ["cd", "My Workspace"]
        );
        assert_eq!(
            split_words(r#"check "a b"\ c"#).unwrap(),
            ["check", "a b c"]
        );
        assert_eq!(split_words("  ").unwrap(), Vec::<String>::new());
        assert!(split_words("cd 'open").is_none());
    }

    #[test]
    fn shell_completion() {
        use rustyline::completion::Completer;
        let h = super::ShellHelper {
            commands: ["check", "check-bag", "status", "explain", "docs"]
                .map(String::from)
                .to_vec(),
            files: rustyline::completion::FilenameCompleter::new(),
        };
        let history = rustyline::history::DefaultHistory::new();
        let ctx = rustyline::Context::new(&history);
        let names = |line: &str| {
            let (start, v) = h.complete(line, line.len(), &ctx).unwrap();
            (
                start,
                v.into_iter().map(|p| p.replacement).collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            names("che"),
            (0, vec!["check".to_string(), "check-bag".to_string()])
        );
        assert_eq!(names("rosfix sta").1, ["status"]);
        assert!(names("explain V00").1.contains(&"V003".to_string()));
        assert_eq!(names("docs edi").1, ["editors"]);
        assert_eq!(names("check --format rd"), (15, vec!["rdjson".to_string()]));
    }
}
