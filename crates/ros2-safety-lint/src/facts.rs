//! The fact model: small typed records extracted from each file.
//!
//! Extractors produce facts; the resolver links them into a launch graph; agents
//! only ever read facts and the graph. Anything that could not be determined
//! statically is represented explicitly (`None` / `Unknown`), never guessed.

use crate::source::{FileId, Loc};
use serde::{Deserialize, Serialize};

// ============================================================================ values

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Double(f64),
    Str(String),
    BoolArray(Vec<bool>),
    IntArray(Vec<i64>),
    DoubleArray(Vec<f64>),
    StrArray(Vec<String>),
    /// `[]`: element type cannot be known.
    EmptyArray,
    /// A nested mapping (only appears transiently while flattening YAML).
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParamType {
    Bool,
    Int,
    Double,
    Str,
    BoolArray,
    IntArray,
    DoubleArray,
    StrArray,
    ByteArray,
    Unknown,
}

impl ParamType {
    pub fn as_str(self) -> &'static str {
        match self {
            ParamType::Bool => "bool",
            ParamType::Int => "integer",
            ParamType::Double => "double",
            ParamType::Str => "string",
            ParamType::BoolArray => "bool array",
            ParamType::IntArray => "integer array",
            ParamType::DoubleArray => "double array",
            ParamType::StrArray => "string array",
            ParamType::ByteArray => "byte array",
            ParamType::Unknown => "unknown",
        }
    }
}

impl ParamValue {
    pub fn ty(&self) -> ParamType {
        match self {
            ParamValue::Bool(_) => ParamType::Bool,
            ParamValue::Int(_) => ParamType::Int,
            ParamValue::Double(_) => ParamType::Double,
            ParamValue::Str(_) => ParamType::Str,
            ParamValue::BoolArray(_) => ParamType::BoolArray,
            ParamValue::IntArray(_) => ParamType::IntArray,
            ParamValue::DoubleArray(_) => ParamType::DoubleArray,
            ParamValue::StrArray(_) => ParamType::StrArray,
            ParamValue::EmptyArray | ParamValue::Unknown => ParamType::Unknown,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            ParamValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(b) => Some(*b),
            ParamValue::Str(s) if s.eq_ignore_ascii_case("true") => Some(true),
            ParamValue::Str(s) if s.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParamValue::Int(i) => Some(*i as f64),
            ParamValue::Double(d) => Some(*d),
            ParamValue::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_str_list(&self) -> Option<Vec<String>> {
        match self {
            ParamValue::StrArray(v) => Some(v.clone()),
            ParamValue::EmptyArray => Some(Vec::new()),
            _ => None,
        }
    }
}

// ============================================================================ packages

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildType {
    AmentCmake,
    AmentPython,
    Cmake,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DepKind {
    Depend,
    Build,
    BuildExport,
    Exec,
    Test,
    BuildTool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub kind: DepKind,
    pub loc: Loc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Executable {
    pub name: String,
    /// Source files (workspace-relative paths) compiled into / implementing it.
    pub sources: Vec<String>,
    pub loc: Loc,
    pub installed: bool,
}

/// Something a package uses that should be declared as a dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageUse {
    pub name: String,
    pub kind: UseKind,
    pub loc: Loc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UseKind {
    /// `find_package(x)` — needs depend/build_depend.
    CmakeFindPackage,
    /// `import x` / `from x import` in non-test Python — needs depend/exec_depend.
    PythonImport,
    /// Launch file references the package at runtime — needs exec_depend.
    LaunchReference,
    /// `import x` in a Python launch file — needs exec_depend (usually present anyway in a
    /// ROS install, so reported as info).
    LaunchImport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    /// Workspace-relative directory containing package.xml ("" for the root).
    pub dir: String,
    pub manifest: FileId,
    pub manifest_name_loc: Loc,
    pub build_type: BuildType,
    pub deps: Vec<Dependency>,
    pub executables: Vec<Executable>,
    /// False if the build file used constructs we could not interpret
    /// (macros, foreach, variables we cannot expand): executable checks are skipped.
    pub executables_complete: bool,
    /// Top-level directories installed into `share/<pkg>` (e.g. "launch", "config").
    pub installed_share_dirs: Vec<String>,
    /// True if share installation could not be determined precisely.
    pub install_rules_complete: bool,
    pub uses: Vec<PackageUse>,
    /// Plugin description XML files exported from package.xml.
    pub plugin_xmls: Vec<String>,
    /// The manifest has dependencies conditional on `$ROS_VERSION == 1` (REP 149): the
    /// package also builds for ROS 1, so its ROS 1 files are intentional.
    #[serde(default)]
    pub ros1_support: bool,
    /// Plugin lookup names/types declared by this package's plugin XMLs.
    pub plugin_classes: Vec<String>,
    /// generate_parameter_library spec files (workspace-relative).
    pub param_lib_specs: Vec<String>,
    /// ament_python: setup.cfg facts.
    pub python: Option<PythonPackageInfo>,
    /// CMakeLists file id, if present.
    pub cmake_file: Option<FileId>,
    /// `rosidl_generate_interfaces(...)` in CMakeLists.txt, if any.
    #[serde(default)]
    pub rosidl: Option<RosidlCall>,
    /// Component libraries never installed: (CMake target, registration call).
    #[serde(default)]
    pub uninstalled_component_libs: Vec<(String, Loc)>,
    /// Literal source paths of `install(FILES|PROGRAMS|DIRECTORY …)`: (workspace-relative
    /// path, install call).
    #[serde(default)]
    pub install_sources: Vec<(String, Loc)>,
    /// ament_python: the literal `packages=[...]` list of setup.py; None when it is computed
    /// (`find_packages()`) or not understood.
    #[serde(default)]
    pub setup_py_packages: Option<(Vec<String>, Loc)>,
}

/// A BehaviorTree.CPP XML file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BtFile {
    pub file: FileId,
    /// Node types used in `<BehaviorTree>`s, with the element location.
    pub nodes: Vec<(String, Loc)>,
    /// `<BehaviorTree ID="…">` defined here.
    pub trees: Vec<String>,
    /// `<SubTree ID="…">` references.
    pub subtree_refs: Vec<(String, Loc)>,
    /// Node types declared in `<TreeNodesModel>`.
    pub declared: Vec<String>,
    /// Uses `<include path=…>` (other trees may come from there).
    pub has_include: bool,
}

/// One `rosidl_generate_interfaces(<target> <files>... [DEPENDENCIES ...])` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosidlCall {
    /// Interface files, relative to the package directory (`msg/Foo.msg`).
    pub files: Vec<String>,
    /// All file arguments were resolved (no unknown variables or globs).
    pub complete: bool,
    /// `SKIP_GROUP_MEMBERSHIP_CHECK` was given.
    pub skip_group_check: bool,
    pub loc: Loc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonPackageInfo {
    pub setup_py: Option<FileId>,
    pub setup_cfg: Option<FileId>,
    pub setup_cfg_install_scripts_ok: bool,
    pub has_console_scripts: bool,
    pub console_scripts_loc: Option<Loc>,
}

// ============================================================================ node implementations

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    Cpp,
    Python,
}

/// Parameter names can be literal or built from a runtime prefix (`plugin_name + ".x"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NamePattern {
    Exact(String),
    /// Unknown prefix followed by a literal suffix (e.g. `<plugin>.max_vel`).
    Suffix(String),
}

impl NamePattern {
    pub fn matches(&self, name: &str) -> bool {
        match self {
            NamePattern::Exact(n) => n == name,
            NamePattern::Suffix(s) => name.ends_with(s.as_str()) && name.len() > s.len(),
        }
    }

    pub fn literal(&self) -> &str {
        match self {
            NamePattern::Exact(s) | NamePattern::Suffix(s) => s,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDecl {
    pub name: NamePattern,
    pub ty: ParamType,
    pub has_default: bool,
    /// The default as written in the source (`1.0`, `"map"`, `{1, 2}`), when it is a
    /// literal; None when there is none or it is computed.
    #[serde(default)]
    pub default_text: Option<String>,
    pub loc: Loc,
    /// Value constraints the node enforces when the parameter is declared (it throws on a
    /// violating override): descriptor ranges, generate_parameter_library validators.
    #[serde(default)]
    pub constraints: Vec<Constraint>,
}

impl Constraint {
    /// The constraint as text when `v` violates it; None when satisfied or not checkable.
    pub fn violated_by(&self, v: &ParamValue) -> Option<String> {
        let c = self;
        let num = match v {
            ParamValue::Int(i) => Some(*i as f64),
            ParamValue::Double(d) => Some(*d),
            _ => None,
        };
        let len = match v {
            ParamValue::Str(s) => Some(s.chars().count()),
            ParamValue::BoolArray(a) => Some(a.len()),
            ParamValue::IntArray(a) => Some(a.len()),
            ParamValue::DoubleArray(a) => Some(a.len()),
            ParamValue::StrArray(a) => Some(a.len()),
            ParamValue::EmptyArray => Some(0),
            _ => None,
        };
        let fmt = |x: f64| if x.fract() == 0.0 && x.abs() < 1e15 { format!("{x:.0}") } else { format!("{x}") };
        match c {
            Constraint::Range { min, max } => num.filter(|x| x < min || x > max).map(|_| format!("the range [{}, {}]", fmt(*min), fmt(*max))),
            Constraint::Lower { bound, inclusive } => {
                num.filter(|x| if *inclusive { x < bound } else { x <= bound }).map(|_| format!("`{} {}`", if *inclusive { ">=" } else { ">" }, fmt(*bound)))
            }
            Constraint::Upper { bound, inclusive } => {
                num.filter(|x| if *inclusive { x > bound } else { x >= bound }).map(|_| format!("`{} {}`", if *inclusive { "<=" } else { "<" }, fmt(*bound)))
            }
            Constraint::OneOf(opts) => match v {
                ParamValue::Str(s) if !opts.contains(s) => Some(format!("one of {}", opts.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(", "))),
                _ => None,
            },
            Constraint::NotEmpty => len.filter(|n| *n == 0).map(|_| "`not empty`".to_string()),
            Constraint::Size { min, max } => len.filter(|n| n < min || n > max).map(|_| match (min, max) {
                (a, b) if a == b => format!("a size of exactly {a}"),
                (0, b) => format!("a size of at most {b}"),
                (a, _) => format!("a size of at least {a}"),
            }),
        }
}
}

impl ParamValue {
    /// Short form for messages.
    pub fn show(&self) -> String {
        let v = self;
        match v {
            ParamValue::Str(s) => format!("`\"{s}\"`"),
            ParamValue::Int(i) => format!("`{i}`"),
            ParamValue::Double(d) => format!("`{d}`"),
            ParamValue::EmptyArray => "`[]`".to_string(),
            other => format!("{} value", other.ty().as_str()),
        }
}
}

/// A value constraint checked by the node at declaration time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Constraint {
    /// Inclusive numeric range (`floating_point_range`/`integer_range`, `bounds<>`).
    Range { min: f64, max: f64 },
    /// `lt<>`, `gt<>`, `lt_eq<>`, `gt_eq<>`: (bound, inclusive).
    Lower { bound: f64, inclusive: bool },
    Upper { bound: f64, inclusive: bool },
    /// `one_of<>`: the value (as text) must be one of these.
    OneOf(Vec<String>),
    /// `not_empty<>` for strings and arrays.
    NotEmpty,
    /// Array length bounds: `size_lt<>`/`size_gt<>`/`fixed_size<>` (min, max inclusive).
    Size { min: usize, max: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Reliability {
    BestEffort,
    Reliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Durability {
    Volatile,
    TransientLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LivelinessKind {
    Automatic,
    ManualByTopic,
}

/// A QoS profile as far as it can be determined statically. `None` fields are unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QosSpec {
    pub reliability: Option<Reliability>,
    pub durability: Option<Durability>,
    pub depth: Option<u32>,
    /// Deadline in nanoseconds; `Some(u64::MAX)` means explicitly infinite.
    pub deadline_ns: Option<u64>,
    pub liveliness: Option<LivelinessKind>,
    pub lease_ns: Option<u64>,
    /// Human-readable origin, e.g. `SensorDataQoS()` or `QoS(10).best_effort()`.
    pub origin: String,
    /// Built from the system-default profile: the policies left unknown here are chosen by
    /// the middleware (its XML profile, else the vendor's defaults), not by ROS.
    #[serde(default)]
    pub system_default: bool,
}

impl QosSpec {
    /// The ROS 2 default profile (reliable, volatile, keep last 10).
    pub fn ros_default(origin: impl Into<String>) -> Self {
        QosSpec {
            reliability: Some(Reliability::Reliable),
            durability: Some(Durability::Volatile),
            depth: Some(10),
            deadline_ns: Some(u64::MAX),
            liveliness: Some(LivelinessKind::Automatic),
            lease_ns: Some(u64::MAX),
            origin: origin.into(),
            system_default: false,
        }
    }

    pub fn sensor_data(origin: impl Into<String>) -> Self {
        QosSpec {
            reliability: Some(Reliability::BestEffort),
            depth: Some(5),
            ..QosSpec::ros_default(origin)
        }
    }

    /// `SystemDefaultsQoS()` / `rmw_qos_profile_system_default`: every policy is left to the
    /// middleware, so nothing is known until a middleware profile says otherwise.
    pub fn system_default(origin: impl Into<String>) -> Self {
        QosSpec {
            origin: origin.into(),
            system_default: true,
            ..Default::default()
        }
    }

    pub fn unknown(origin: impl Into<String>) -> Self {
        QosSpec {
            origin: origin.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EndpointKind {
    Publisher,
    Subscription,
    ServiceServer,
    ServiceClient,
    ActionServer,
    ActionClient,
}

impl EndpointKind {
    pub fn is_topic(self) -> bool {
        matches!(self, EndpointKind::Publisher | EndpointKind::Subscription)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EndpointKind::Publisher => "publisher",
            EndpointKind::Subscription => "subscription",
            EndpointKind::ServiceServer => "service server",
            EndpointKind::ServiceClient => "service client",
            EndpointKind::ActionServer => "action server",
            EndpointKind::ActionClient => "action client",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub kind: EndpointKind,
    /// The name exactly as written in code, if it is a literal.
    pub name: Option<String>,
    pub msg_type: Option<String>,
    pub qos: QosSpec,
    pub qos_overrides_enabled: bool,
    /// Variable / member the endpoint (client) is stored in, used by the executor agent.
    pub var: Option<String>,
    /// Callback group variable passed at creation (clients/subscriptions/timers).
    pub group: Option<String>,
    /// Callback handling incoming data (subscriptions/services).
    pub callback: Option<CallbackTarget>,
    pub loc: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallbackTarget {
    /// Byte range of a lambda / inline function body.
    Inline { start: usize, end: usize },
    /// A method or free function referenced by name.
    Function(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallbackKind {
    Subscription,
    Timer,
    Service,
    Action,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Callback {
    pub kind: CallbackKind,
    pub target: CallbackTarget,
    pub group: Option<String>,
    /// Timer period in milliseconds, when written as a literal duration.
    #[serde(default)]
    pub period_ms: Option<f64>,
    pub loc: Loc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupKind {
    MutuallyExclusive,
    Reentrant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackGroupDecl {
    pub var: String,
    pub kind: GroupKind,
    /// True if `automatically_add_to_executor_with_node` was passed as false.
    pub not_auto_added: bool,
    pub loc: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockingKind {
    /// spin_until_future_complete / spin_some / spin on `executor` (None = node / global spin).
    Spin {
        executor: Option<String>,
    },
    /// `.get()` / `.wait()` / `.wait_for()` / `.result()` on a future returned by
    /// an async request from `client` (variable name).
    FutureWait {
        client: Option<String>,
    },
    /// rclpy synchronous `client.call(req)`.
    SyncCall {
        client: Option<String>,
    },
    Sleep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockingCall {
    pub kind: BlockingKind,
    pub loc: Loc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    /// Unqualified name (method or free function).
    pub name: String,
    pub class: Option<String>,
    pub start: usize,
    pub end: usize,
    /// Names of methods/functions called from the body (unqualified).
    pub calls: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutorKind {
    SingleThreaded,
    MultiThreaded,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorUse {
    pub kind: ExecutorKind,
    /// Variable holding the executor, if any.
    pub var: Option<String>,
    pub loc: Loc,
}

/// Facts about one ROS node implementation (a class deriving from Node, or a file
/// that creates a node directly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeImpl {
    pub file: FileId,
    pub lang: Lang,
    pub class_name: Option<String>,
    /// Base class as written (C++ base clause / Python base), if known.
    pub base: Option<String>,
    pub default_name: Option<String>,
    pub params: Vec<ParamDecl>,
    /// The node may accept parameters we cannot enumerate (dynamic names,
    /// allow_undeclared_parameters, generic helpers): P003 must stay silent.
    pub params_open: bool,
    pub endpoints: Vec<Endpoint>,
    pub callbacks: Vec<Callback>,
    pub groups: Vec<CallbackGroupDecl>,
    /// Callback groups added to a dedicated executor (the "callback_group_executor" pattern).
    pub dedicated_groups: Vec<String>,
    pub loc: Loc,
}

/// Per-source-file facts for C++/Python files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeFacts {
    pub nodes: Vec<NodeImpl>,
    pub functions: Vec<FunctionInfo>,
    pub blocking: Vec<BlockingCall>,
    pub executors: Vec<ExecutorUse>,
    /// Packages imported/included (for dependency checks).
    pub imports: Vec<(String, Loc)>,
    /// Every call of an unqualified/member function: (callee name, byte offset).
    /// Lets the executor agent follow a lambda callback into the methods it calls.
    pub call_sites: Vec<(String, usize)>,
    /// Parameters declared through a node handle outside a known node class:
    /// (enclosing class, declaration — None if the name is dynamic). The resolver
    /// attributes them to a linked node class, or marks the package's nodes open.
    pub class_params: Vec<(Option<String>, Option<ParamDecl>)>,
    /// The file loads plugins (pluginlib) — the host node's parameters are open.
    pub loads_plugins: bool,
    /// Names of classes defined (with a body) in this file.
    pub classes_defined: Vec<String>,
    /// Namespaces passed to generate_parameter_library listeners (`ParamListener(node, "ns")`):
    /// the library's parameters are declared as `ns.<name>`. None: the prefix is not known.
    #[serde(default)]
    pub param_prefixes: Vec<Option<String>>,
}

// ============================================================================ launch

/// A launch-time value: literal text, a launch argument reference, or a mix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LVal {
    Lit(String),
    Arg(String),
    /// `$(find-pkg-share pkg)` / `FindPackageShare('pkg')` / `get_package_share_directory('pkg')`.
    PkgShare(String),
    /// `$(find-pkg-prefix pkg)`
    PkgPrefix(String),
    /// Several parts concatenated (`PathJoinSubstitution` joins with '/').
    Concat(Vec<LVal>),
    PathJoin(Vec<LVal>),
    Unknown,
}

impl LVal {
    pub fn lit(s: impl Into<String>) -> Self {
        LVal::Lit(s.into())
    }

    pub fn as_lit(&self) -> Option<&str> {
        match self {
            LVal::Lit(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LaunchParamSource {
    /// A parameter file. `root_key` is set when wrapped in nav2's `RewrittenYaml`.
    File {
        path: LVal,
        root_key: Option<LVal>,
        loc: Loc,
    },
    Inline {
        name: String,
        value: LVal,
        typed: Option<ParamValue>,
        loc: Loc,
    },
    /// Something we cannot interpret (a variable, a function call).
    Opaque { loc: Loc },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LaunchNodeKind {
    Node,
    LifecycleNode,
    Composable,
    Container,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchNodeDecl {
    pub kind: LaunchNodeKind,
    pub pkg: LVal,
    pub exec: LVal,
    /// For composable nodes: the plugin class (`ns::Class`).
    pub plugin: Option<LVal>,
    pub name: Option<LVal>,
    pub namespace: Option<LVal>,
    pub params: Vec<LaunchParamSource>,
    pub remaps: Vec<(LVal, LVal, Loc)>,
    /// `--enclave` passed in ros_arguments / `ROS_SECURITY_ENCLAVE_OVERRIDE`.
    pub enclave: Option<LVal>,
    /// Extra arguments (`arguments=[...]` / `args=`), e.g. rviz `-d file`.
    pub arguments: Vec<LVal>,
    pub conditional: bool,
    pub loc: Loc,
}

// Node declarations dominate launch files; boxing them would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LaunchItem {
    Node(LaunchNodeDecl),
    Include {
        path: LVal,
        args: Vec<(String, LVal)>,
        conditional: bool,
        loc: Loc,
    },
    /// A group scope (namespace push / scoped parameters / remaps).
    Group {
        namespace: Option<LVal>,
        items: Vec<LaunchItem>,
        scoped: bool,
        loc: Loc,
    },
    /// `SetParameter` / `<set_parameter>`: applies to all following nodes in scope.
    SetParam {
        name: String,
        value: LVal,
        loc: Loc,
    },
    SetRemap {
        from: LVal,
        to: LVal,
        loc: Loc,
    },
    SetEnv {
        name: String,
        value: LVal,
        loc: Loc,
    },
    /// `DeclareLaunchArgument` within a scope that also declares arguments later.
    Arg {
        name: String,
        default: Option<LVal>,
        loc: Loc,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchFile {
    pub file: FileId,
    pub items: Vec<LaunchItem>,
    /// Parts of the file could not be interpreted statically.
    pub partial: bool,
}

// ============================================================================ parameter files

/// One node section of a parameter YAML file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamSection {
    /// The node key path joined with '/', as written (e.g. `controller_server`, `/**`, `ns/node`).
    pub node_key: String,
    pub key_loc: Loc,
    /// Flattened (dotted) parameter names and values.
    pub params: Vec<ParamEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamEntry {
    pub name: String,
    pub value: ParamValue,
    pub key_loc: Loc,
    pub value_loc: Loc,
    /// Raw scalar text for scalars (used for type fixes like `1` → `1.0`).
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamFile {
    pub file: FileId,
    pub sections: Vec<ParamSection>,
    /// Node sections that are missing `ros__parameters` (P002).
    pub missing_ros_parameters: Vec<MissingRosParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingRosParams {
    pub node_key: String,
    pub key_loc: Loc,
    /// A key that looks like a misspelling of `ros__parameters`, if any.
    pub misspelled: Option<(String, Loc)>,
}

// ============================================================================ robot description

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrdfJoint {
    pub name: String,
    pub joint_type: String,
    /// `<parent link=…>` and `<child link=…>` (evaluated; may contain unresolved `${…}`).
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub child: Option<String>,
    pub loc: Loc,
    /// The `<limit>` element, if any.
    #[serde(default)]
    pub limit: Option<UrdfLimit>,
    /// The element contains xacro constructs, so its children are not fully known.
    #[serde(default)]
    pub opaque: bool,
}

/// A number in the robot description: None when the attribute or element is missing,
/// Some(None) when present but not evaluable statically.
pub type UrdfNumber = Option<Option<f64>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrdfLimit {
    pub loc: Loc,
    pub lower: UrdfNumber,
    pub upper: UrdfNumber,
    pub effort: UrdfNumber,
    pub velocity: UrdfNumber,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrdfInertial {
    pub link: String,
    pub loc: Loc,
    pub mass: UrdfNumber,
    /// `<inertia>` ixx, ixy, ixz, iyy, iyz, izz; None when the element is missing.
    pub inertia: Option<Vec<UrdfNumber>>,
}

/// A `<mesh filename=…>` of a visual or collision geometry (evaluated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrdfMesh {
    pub uri: String,
    pub loc: Loc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlJoint {
    pub name: String,
    pub command_interfaces: Vec<String>,
    pub state_interfaces: Vec<String>,
    pub loc: Loc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ros2ControlBlock {
    pub name: String,
    pub hardware_plugin: Option<String>,
    pub joints: Vec<ControlJoint>,
    pub sensors: Vec<String>,
    pub gpios: Vec<String>,
    pub loc: Loc,
}

/// A robot description (URDF or expanded top-level xacro).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotModel {
    pub file: FileId,
    pub links: Vec<String>,
    pub joints: Vec<UrdfJoint>,
    pub control: Vec<Ros2ControlBlock>,
    /// Names containing unresolved `${…}`/`$(…)` were seen: name comparisons must be lenient.
    pub partial: bool,
    /// `<inertial>` blocks of links whose content is fully known.
    #[serde(default)]
    pub inertials: Vec<UrdfInertial>,
    #[serde(default)]
    pub meshes: Vec<UrdfMesh>,
}

// ============================================================================ security

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceRule {
    pub key: String,
    pub value: String,
    pub loc: Loc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionGrant {
    pub subject: String,
    pub default_allow: Option<Loc>,
    pub wildcard_rules: Vec<(String, Loc)>,
    pub not_before: Option<(String, Loc)>,
    pub not_after: Option<(String, Loc)>,
    pub loc: Loc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyProfile {
    pub enclave: String,
    pub ns: String,
    pub node: String,
    pub publish: Vec<String>,
    pub subscribe: Vec<String>,
    pub loc: Loc,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityFacts {
    pub governance_weak: Vec<GovernanceRule>,
    pub grants: Vec<PermissionGrant>,
    pub policy_profiles: Vec<PolicyProfile>,
    pub enclaves: Vec<String>,
}

// ============================================================================ rviz

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RvizSubscription {
    pub topic: String,
    pub display: String,
    pub qos: QosSpec,
    pub loc: Loc,
}
