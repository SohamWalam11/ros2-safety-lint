use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Unique identifier for elements in the Cross-Language IR
pub type ElementId = String;

/// File origin tracking for traceability
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileOrigin {
    pub file_path: PathBuf,
    pub file_type: FileType,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub byte_range: Option<std::ops::Range<usize>>,
}

/// Supported file types in ROS 2 configuration ecosystem
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FileType {
    XML,
    YAML, 
    Python,
    Cpp,
    Header,
    URDF,
    Xacro,
}

/// Unified Cross-Language Intermediate Representation
/// 
/// This structure captures all ROS 2 semantic elements across multiple file types,
/// enabling cross-file dependency analysis and Shadow Configuration Drift detection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrossLanguageIR {
    /// ROS 2 node definitions from all sources
    pub nodes: Vec<NodeDefinition>,
    /// Parameter definitions and their sources
    pub parameters: Vec<ParameterDefinition>,
    /// Topic definitions with type information
    pub topics: Vec<TopicDefinition>,
    /// Service definitions and interfaces
    pub services: Vec<ServiceDefinition>,
    /// QoS profile definitions
    pub qos_profiles: Vec<QoSProfile>,
    /// Security policy definitions
    pub security_policies: Vec<SecurityPolicy>,
    /// Dependency graph between elements
    pub dependencies: DependencyGraph,
    /// File origin mapping for all elements
    pub file_origins: HashMap<ElementId, FileOrigin>,
    /// Launch file configurations
    pub launch_configs: Vec<LaunchConfiguration>,
    /// Physical robot model information
    pub physical_models: Vec<PhysicalModel>,
}

/// ROS 2 node definition across all configuration sources
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeDefinition {
    pub id: ElementId,
    pub name: String,
    pub package: Option<String>,
    pub executable: Option<String>,
    pub namespace: Option<String>,
    pub node_type: NodeType,
    pub lifecycle: LifecycleConfiguration,
    pub parameters: Vec<ElementId>,
    pub topics: Vec<ElementId>,
    pub services: Vec<ElementId>,
    pub remappings: Vec<Remapping>,
    pub file_origins: Vec<ElementId>,
}

/// Types of ROS 2 nodes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeType {
    Regular,
    Lifecycle,
    Component,
    Composable,
}

/// Lifecycle configuration for managed nodes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifecycleConfiguration {
    pub is_managed: bool,
    pub auto_start: bool,
    pub state_transitions: Vec<StateTransition>,
}

/// State transition in lifecycle management
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateTransition {
    pub from_state: String,
    pub to_state: String,
    pub trigger: String,
}

/// Parameter definition with type and source information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterDefinition {
    pub id: ElementId,
    pub name: String,
    pub namespace: Option<String>,
    pub value: ParameterValue,
    pub parameter_type: ParameterType,
    pub source: ParameterSource,
    pub override_chain: Vec<ElementId>,
}

/// Parameter value types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParameterValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<ParameterValue>),
    Object(HashMap<String, ParameterValue>),
}

/// Parameter type classification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParameterType {
    Configuration,
    QoSOverride,
    SecuritySetting,
    PhysicalConstraint,
    RuntimeTuning,
}

/// Source of parameter definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParameterSource {
    YAMLFile(PathBuf),
    LaunchFile(PathBuf),
    SourceCode(PathBuf),
    CommandLine,
    Environment,
}

/// Topic definition with QoS and type information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicDefinition {
    pub id: ElementId,
    pub name: String,
    pub message_type: String,
    pub publishers: Vec<ElementId>,
    pub subscribers: Vec<ElementId>,
    pub qos_profile: Option<ElementId>,
    pub security_policy: Option<ElementId>,
}

/// Service definition with interface information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceDefinition {
    pub id: ElementId,
    pub name: String,
    pub service_type: String,
    pub servers: Vec<ElementId>,
    pub clients: Vec<ElementId>,
    pub qos_profile: Option<ElementId>,
}

/// QoS profile definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QoSProfile {
    pub id: ElementId,
    pub name: Option<String>,
    pub reliability: QoSReliability,
    pub durability: QoSDurability,
    pub history: QoSHistory,
    pub depth: Option<u32>,
    pub deadline: Option<u64>,
    pub lifespan: Option<u64>,
    pub liveliness: QoSLiveliness,
    pub liveliness_lease_duration: Option<u64>,
}

/// QoS reliability policy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QoSReliability {
    SystemDefault,
    Reliable,
    BestEffort,
}

/// QoS durability policy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QoSDurability {
    SystemDefault,
    TransientLocal,
    Volatile,
}

/// QoS history policy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QoSHistory {
    SystemDefault,
    KeepLast(u32),
    KeepAll,
}

/// QoS liveliness policy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QoSLiveliness {
    SystemDefault,
    Automatic,
    ManualByNode,
    ManualByTopic,
}

/// Security policy definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityPolicy {
    pub id: ElementId,
    pub governance: GovernanceSettings,
    pub permissions: Vec<Permission>,
    pub certificates: CertificateSettings,
}

/// Governance settings for SROS2
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GovernanceSettings {
    pub domain_id: Option<u32>,
    pub rtps_protection_kind: ProtectionKind,
    pub discovery_protection_kind: ProtectionKind,
    pub liveliness_protection_kind: ProtectionKind,
}

/// Protection kinds for SROS2
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProtectionKind {
    None,
    Sign,
    Encrypt,
}

/// Permission definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Permission {
    pub subject_name: String,
    pub allow_rule: AllowRule,
}

/// Allow rule for permissions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllowRule {
    pub domains: Vec<u32>,
    pub publish: Vec<String>,
    pub subscribe: Vec<String>,
    pub call: Vec<String>,
    pub reply: Vec<String>,
}

/// Certificate settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CertificateSettings {
    pub ca_cert_path: Option<PathBuf>,
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    pub keystore_path: Option<PathBuf>,
}

/// Topic/service remapping
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Remapping {
    pub from: String,
    pub to: String,
}

/// Launch file configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LaunchConfiguration {
    pub id: ElementId,
    pub launch_file: PathBuf,
    pub arguments: Vec<LaunchArgument>,
    pub conditions: Vec<LaunchCondition>,
    pub includes: Vec<PathBuf>,
}

/// Launch file argument
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LaunchArgument {
    pub name: String,
    pub default_value: Option<String>,
    pub description: Option<String>,
}

/// Launch file condition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LaunchCondition {
    pub condition_type: String,
    pub expression: String,
}

/// Physical robot model information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhysicalModel {
    pub id: ElementId,
    pub model_file: PathBuf,
    pub joints: Vec<Joint>,
    pub links: Vec<Link>,
    pub safety_constraints: Vec<SafetyConstraint>,
}

/// Joint definition in URDF
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Joint {
    pub name: String,
    pub joint_type: JointType,
    pub limits: Option<JointLimits>,
    pub parent_link: String,
    pub child_link: String,
}

/// Joint types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JointType {
    Fixed,
    Revolute,
    Continuous,
    Prismatic,
    Floating,
    Planar,
}

/// Joint limits
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JointLimits {
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub effort: Option<f64>,
    pub velocity: Option<f64>,
}

/// Link definition in URDF
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Link {
    pub name: String,
    pub inertial: Option<Inertial>,
    pub visual: Vec<Visual>,
    pub collision: Vec<Collision>,
}

/// Inertial properties
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Inertial {
    pub mass: f64,
    pub center_of_mass: [f64; 3],
    pub inertia_matrix: [f64; 6],
}

/// Visual element
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Visual {
    pub name: Option<String>,
    pub geometry: Geometry,
}

/// Collision element
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Collision {
    pub name: Option<String>,
    pub geometry: Geometry,
}

/// Geometry definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Geometry {
    Box { size: [f64; 3] },
    Cylinder { radius: f64, length: f64 },
    Sphere { radius: f64 },
    Mesh { filename: PathBuf, scale: Option<[f64; 3]> },
}

/// Safety constraint
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafetyConstraint {
    pub constraint_type: ConstraintType,
    pub description: String,
    pub severity: ConstraintSeverity,
}

/// Constraint types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConstraintType {
    JointLimit,
    VelocityLimit,
    AccelerationLimit,
    CollisionAvoidance,
    WorkspaceLimit,
}

/// Constraint severity levels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConstraintSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Dependency graph for cross-file analysis
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DependencyGraph {
    pub nodes: HashSet<ElementId>,
    pub edges: HashSet<(ElementId, ElementId)>,
    pub dependency_types: HashMap<(ElementId, ElementId), DependencyType>,
}

/// Types of dependencies between configuration elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DependencyType {
    ParameterOverride,
    TopicConnection,
    ServiceConnection,
    LaunchInclusion,
    NodeLifecycleDependency,
    QoSProfileUsage,
    SecurityPolicyApplication,
    PhysicalModelReference,
}

impl CrossLanguageIR {
    /// Create a new empty IR
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node definition to the IR
    pub fn add_node(&mut self, node: NodeDefinition) -> ElementId {
        let id = node.id.clone();
        self.nodes.push(node);
        id
    }

    /// Add a parameter definition to the IR
    pub fn add_parameter(&mut self, parameter: ParameterDefinition) -> ElementId {
        let id = parameter.id.clone();
        self.parameters.push(parameter);
        id
    }

    /// Add a topic definition to the IR
    pub fn add_topic(&mut self, topic: TopicDefinition) -> ElementId {
        let id = topic.id.clone();
        self.topics.push(topic);
        id
    }

    /// Add a service definition to the IR
    pub fn add_service(&mut self, service: ServiceDefinition) -> ElementId {
        let id = service.id.clone();
        self.services.push(service);
        id
    }

    /// Add a QoS profile to the IR
    pub fn add_qos_profile(&mut self, profile: QoSProfile) -> ElementId {
        let id = profile.id.clone();
        self.qos_profiles.push(profile);
        id
    }

    /// Add a security policy to the IR
    pub fn add_security_policy(&mut self, policy: SecurityPolicy) -> ElementId {
        let id = policy.id.clone();
        self.security_policies.push(policy);
        id
    }

    /// Add a launch configuration to the IR
    pub fn add_launch_config(&mut self, config: LaunchConfiguration) -> ElementId {
        let id = config.id.clone();
        self.launch_configs.push(config);
        id
    }

    /// Add a physical model to the IR
    pub fn add_physical_model(&mut self, model: PhysicalModel) -> ElementId {
        let id = model.id.clone();
        self.physical_models.push(model);
        id
    }

    /// Add a dependency between two elements
    pub fn add_dependency(&mut self, from: ElementId, to: ElementId, dep_type: DependencyType) {
        self.dependencies.nodes.insert(from.clone());
        self.dependencies.nodes.insert(to.clone());
        self.dependencies.edges.insert((from.clone(), to.clone()));
        self.dependencies.dependency_types.insert((from, to), dep_type);
    }

    /// Set file origin for an element
    pub fn set_file_origin(&mut self, element_id: ElementId, origin: FileOrigin) {
        self.file_origins.insert(element_id, origin);
    }

    /// Get all elements that depend on the given element
    pub fn get_dependents(&self, element_id: &ElementId) -> Vec<&ElementId> {
        self.dependencies
            .edges
            .iter()
            .filter_map(|(from, to)| {
                if to == element_id {
                    Some(from)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all elements that the given element depends on
    pub fn get_dependencies(&self, element_id: &ElementId) -> Vec<&ElementId> {
        self.dependencies
            .edges
            .iter()
            .filter_map(|(from, to)| {
                if from == element_id {
                    Some(to)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check for circular dependencies
    pub fn has_circular_dependencies(&self) -> bool {
        // Simplified cycle detection - could be enhanced with proper topological sort
        for node in &self.dependencies.nodes {
            if self.has_path_to_self(node, &mut HashSet::new()) {
                return true;
            }
        }
        false
    }

    fn has_path_to_self(&self, start: &ElementId, visited: &mut HashSet<ElementId>) -> bool {
        if visited.contains(start) {
            return true;
        }
        visited.insert(start.clone());
        
        for dep in self.get_dependencies(start) {
            if self.has_path_to_self(dep, visited) {
                return true;
            }
        }
        
        visited.remove(start);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_create_empty_ir() {
        let ir = CrossLanguageIR::new();
        assert!(ir.nodes.is_empty());
        assert!(ir.parameters.is_empty());
        assert!(ir.topics.is_empty());
    }

    #[test]
    fn test_add_node() {
        let mut ir = CrossLanguageIR::new();
        let node = NodeDefinition {
            id: "test_node".to_string(),
            name: "test".to_string(),
            package: Some("test_pkg".to_string()),
            executable: Some("test_exe".to_string()),
            namespace: None,
            node_type: NodeType::Regular,
            lifecycle: LifecycleConfiguration {
                is_managed: false,
                auto_start: false,
                state_transitions: vec![],
            },
            parameters: vec![],
            topics: vec![],
            services: vec![],
            remappings: vec![],
            file_origins: vec![],
        };
        
        let id = ir.add_node(node);
        assert_eq!(id, "test_node");
        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.nodes[0].name, "test");
    }

    #[test]
    fn test_dependency_management() {
        let mut ir = CrossLanguageIR::new();
        
        // Add dependency
        ir.add_dependency(
            "node1".to_string(),
            "param1".to_string(),
            DependencyType::ParameterOverride,
        );
        
        assert!(ir.dependencies.nodes.contains("node1"));
        assert!(ir.dependencies.nodes.contains("param1"));
        assert!(ir.dependencies.edges.contains(&("node1".to_string(), "param1".to_string())));
        
        // Test dependency queries
        let deps = ir.get_dependencies(&"node1".to_string());
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "param1");
        
        let dependents = ir.get_dependents(&"param1".to_string());
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0], "node1");
    }

    #[test]
    fn test_file_origin_tracking() {
        let mut ir = CrossLanguageIR::new();
        let origin = FileOrigin {
            file_path: PathBuf::from("test.yaml"),
            file_type: FileType::YAML,
            line: Some(10),
            column: Some(5),
            byte_range: Some(100..200),
        };
        
        ir.set_file_origin("test_element".to_string(), origin.clone());
        
        let retrieved_origin = ir.file_origins.get("test_element").unwrap();
        assert_eq!(retrieved_origin.file_path, PathBuf::from("test.yaml"));
        assert_eq!(retrieved_origin.file_type, FileType::YAML);
        assert_eq!(retrieved_origin.line, Some(10));
    }

    #[test]
    fn test_circular_dependency_detection() {
        let mut ir = CrossLanguageIR::new();
        
        // Create circular dependency: A -> B -> A
        ir.add_dependency("A".to_string(), "B".to_string(), DependencyType::ParameterOverride);
        ir.add_dependency("B".to_string(), "A".to_string(), DependencyType::ParameterOverride);
        
        assert!(ir.has_circular_dependencies());
    }

    #[test] 
    fn test_no_circular_dependencies() {
        let mut ir = CrossLanguageIR::new();
        
        // Create acyclic dependency: A -> B -> C
        ir.add_dependency("A".to_string(), "B".to_string(), DependencyType::ParameterOverride);
        ir.add_dependency("B".to_string(), "C".to_string(), DependencyType::ParameterOverride);
        
        assert!(!ir.has_circular_dependencies());
    }
}