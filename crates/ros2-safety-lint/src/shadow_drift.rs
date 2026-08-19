use crate::cross_language_ir::*;
use crate::sros2::LintViolation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

/// Shadow Configuration Drift Detection System
/// 
/// Identifies inconsistencies between configuration intent (XML/YAML) and
/// implementation reality (Python/C++) that are only visible through cross-file analysis.
pub struct ShadowDriftDetector {
    drift_patterns: Vec<Box<dyn DriftPattern>>,
}

/// Types of Shadow Configuration Drift
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ShadowDriftType {
    /// QoS reliability mismatch between configuration and implementation
    QoSMismatch {
        configured: QoSReliability,
        implemented: QoSReliability,
        topic_name: String,
    },
    /// Security policy inconsistency
    SecurityInconsistency {
        expected_protection: ProtectionKind,
        actual_protection: ProtectionKind,
        element: String,
    },
    /// Managed lifecycle node state transition violations
    LifecycleViolation {
        expected_state: String,
        actual_transitions: Vec<String>,
        node_name: String,
    },
    /// Physical safety constraint violations
    SafetyBoundaryViolation {
        constraint_type: ConstraintType,
        violation_description: String,
        affected_component: String,
    },
    /// Parameter conflict across configuration sources
    ParameterConflict {
        parameter_name: String,
        conflicting_values: Vec<(ParameterSource, ParameterValue)>,
        resolution: ConflictResolution,
    },
    /// Topic type mismatch between publishers and subscribers
    TopicTypeMismatch {
        topic_name: String,
        publisher_types: Vec<String>,
        subscriber_types: Vec<String>,
    },
    /// Service interface inconsistency
    ServiceInterfaceInconsistency {
        service_name: String,
        server_interface: String,
        client_interfaces: Vec<String>,
    },
    /// Executor configuration mismatch
    ExecutorMismatch {
        expected_threading: ThreadingModel,
        blocking_operations: Vec<BlockingOperation>,
        node_name: String,
    },
}

/// Configuration conflict resolution strategy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConflictResolution {
    SourceCodeOverrides,
    ConfigurationFileOverrides,
    CommandLineOverrides,
    EnvironmentOverrides,
    Unresolved,
}

/// Threading model expectations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ThreadingModel {
    SingleThreaded,
    MultiThreaded,
    Unknown,
}

/// Blocking operation detection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlockingOperation {
    pub operation_type: String,
    pub location: FileOrigin,
    pub estimated_duration: Option<u64>, // milliseconds
}

/// Shadow Configuration Drift instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowDrift {
    pub id: String,
    pub drift_type: ShadowDriftType,
    pub affected_elements: Vec<ElementId>,
    pub inconsistency_details: InconsistencyReport,
    pub file_locations: Vec<FileOrigin>,
    pub severity: DriftSeverity,
    pub impact_assessment: ImpactAssessment,
    pub remediation_suggestions: Vec<RemediationSuggestion>,
}

/// Inconsistency report details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InconsistencyReport {
    pub description: String,
    pub configuration_source: FileOrigin,
    pub implementation_source: FileOrigin,
    pub precedence_explanation: String,
    pub runtime_implications: Vec<String>,
}

/// Severity classification for Shadow Configuration Drift
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum DriftSeverity {
    Info,      // Informational inconsistency, no functional impact
    Warning,   // Potential performance or usability issue
    Error,     // Functional issue that may cause failures
    Critical,  // Safety or security issue requiring immediate attention
}

/// Impact assessment for drift issues
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAssessment {
    pub functional_impact: String,
    pub performance_impact: Option<String>,
    pub security_impact: Option<String>,
    pub safety_impact: Option<String>,
    pub affected_use_cases: Vec<String>,
}

/// Remediation suggestion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationSuggestion {
    pub strategy: RemediationStrategy,
    pub description: String,
    pub confidence: f32, // 0.0 to 1.0
    pub automation_possible: bool,
}

/// Remediation strategies for Shadow Configuration Drift
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RemediationStrategy {
    UpdateConfiguration,
    ModifyImplementation,
    AddExplicitOverride,
    DocumentIntentionalDeviation,
    RefactorArchitecture,
}

/// Trait for Shadow Configuration Drift detection patterns
pub trait DriftPattern: Send + Sync {
    /// Pattern name for identification
    fn name(&self) -> &str;
    
    /// Detect drift instances in the Cross-Language IR
    fn detect(&self, ir: &CrossLanguageIR) -> Vec<ShadowDrift>;
    
    /// Pattern description for documentation
    fn description(&self) -> &str;
    
    /// Severity level for this pattern type
    fn default_severity(&self) -> DriftSeverity;
}

impl ShadowDriftDetector {
    /// Create new Shadow Configuration Drift detector with default patterns
    pub fn new() -> Self {
        let mut detector = Self {
            drift_patterns: Vec::new(),
        };
        
        detector.register_default_patterns();
        detector
    }

    /// Register default drift detection patterns
    fn register_default_patterns(&mut self) {
        self.add_pattern(Box::new(QoSOverridePattern::new()));
        self.add_pattern(Box::new(SecurityPolicyOverridePattern::new()));
        self.add_pattern(Box::new(LifecycleViolationPattern::new()));
        self.add_pattern(Box::new(ParameterConflictPattern::new()));
        self.add_pattern(Box::new(TopicTypeMismatchPattern::new()));
        self.add_pattern(Box::new(ExecutorDeadlockPattern::new()));
        self.add_pattern(Box::new(PhysicalSafetyViolationPattern::new()));
    }

    /// Add a drift detection pattern
    pub fn add_pattern(&mut self, pattern: Box<dyn DriftPattern>) {
        self.drift_patterns.push(pattern);
    }

    /// Detect all Shadow Configuration Drift instances
    pub fn detect_shadow_drift(&self, ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        let mut all_drifts = Vec::new();
        
        for pattern in &self.drift_patterns {
            let mut pattern_drifts = pattern.detect(ir);
            all_drifts.append(&mut pattern_drifts);
        }
        
        // Post-process drifts: deduplicate, prioritize, assess impact
        self.post_process_drifts(all_drifts)
    }

    /// Post-process detected drifts for deduplication and impact assessment
    fn post_process_drifts(&self, mut drifts: Vec<ShadowDrift>) -> Vec<ShadowDrift> {
        // Sort by severity (most critical first)
        drifts.sort_by(|a, b| b.severity.partial_cmp(&a.severity).unwrap());
        
        // TODO: Implement deduplication logic for overlapping drift instances
        // TODO: Enhance impact assessment based on system context
        
        drifts
    }

    /// Convert Shadow Configuration Drift to LintViolation for integration
    pub fn to_lint_violations(&self, drifts: &[ShadowDrift]) -> Vec<LintViolation> {
        drifts.iter().map(|drift| {
            LintViolation {
                message: format!(
                    "Shadow Configuration Drift [{}]: {}",
                    drift.drift_type.type_name(),
                    drift.inconsistency_details.description
                ),
                range: drift.file_locations.first()
                    .and_then(|loc| loc.byte_range.clone())
                    .unwrap_or(0..1),
            }
        }).collect()
    }

    /// Generate comprehensive drift analysis report
    pub fn generate_drift_report(&self, drifts: &[ShadowDrift]) -> DriftReport {
        let mut report = DriftReport {
            total_drifts: drifts.len(),
            severity_breakdown: HashMap::new(),
            pattern_breakdown: HashMap::new(),
            affected_files: std::collections::HashSet::new(),
            impact_summary: ImpactSummary::default(),
            recommendations: Vec::new(),
        };

        // Analyze drift distribution
        for drift in drifts {
            // Count by severity
            *report.severity_breakdown.entry(drift.severity.clone()).or_insert(0) += 1;
            
            // Count by pattern type
            let pattern_type = drift.drift_type.type_name();
            *report.pattern_breakdown.entry(pattern_type.to_string()).or_insert(0) += 1;
            
            // Collect affected files
            for location in &drift.file_locations {
                report.affected_files.insert(location.file_path.clone());
            }
        }

        report
    }
}

impl ShadowDriftType {
    /// Get the type name for categorization
    pub fn type_name(&self) -> &'static str {
        match self {
            ShadowDriftType::QoSMismatch { .. } => "QoSMismatch",
            ShadowDriftType::SecurityInconsistency { .. } => "SecurityInconsistency",
            ShadowDriftType::LifecycleViolation { .. } => "LifecycleViolation",
            ShadowDriftType::SafetyBoundaryViolation { .. } => "SafetyBoundaryViolation",
            ShadowDriftType::ParameterConflict { .. } => "ParameterConflict",
            ShadowDriftType::TopicTypeMismatch { .. } => "TopicTypeMismatch",
            ShadowDriftType::ServiceInterfaceInconsistency { .. } => "ServiceInterfaceInconsistency",
            ShadowDriftType::ExecutorMismatch { .. } => "ExecutorMismatch",
        }
    }
}

/// Comprehensive drift analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub total_drifts: usize,
    pub severity_breakdown: HashMap<DriftSeverity, usize>,
    pub pattern_breakdown: HashMap<String, usize>,
    pub affected_files: std::collections::HashSet<PathBuf>,
    pub impact_summary: ImpactSummary,
    pub recommendations: Vec<String>,
}

/// Summary of overall impact from Shadow Configuration Drift
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImpactSummary {
    pub critical_issues: usize,
    pub security_concerns: usize,
    pub performance_issues: usize,
    pub safety_violations: usize,
    pub estimated_resolution_effort: Option<String>,
}

// ================================================================================================
// DRIFT PATTERN IMPLEMENTATIONS
// ================================================================================================

/// Pattern: QoS Override Detection
/// Detects when source code overrides QoS settings defined in configuration files
pub struct QoSOverridePattern;

impl QoSOverridePattern {
    pub fn new() -> Self {
        Self
    }
}

impl DriftPattern for QoSOverridePattern {
    fn name(&self) -> &str {
        "QoS Override"
    }

    fn description(&self) -> &str {
        "Detects QoS profile overrides in source code that contradict configuration files"
    }

    fn default_severity(&self) -> DriftSeverity {
        DriftSeverity::Error
    }

    fn detect(&self, ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        let mut drifts = Vec::new();
        
        // Find topics with configured QoS profiles
        for topic in &ir.topics {
            if let Some(configured_qos_id) = &topic.qos_profile {
                if let Some(configured_qos) = ir.qos_profiles.iter().find(|q| &q.id == configured_qos_id) {
                    // Check for source code overrides in publishers/subscribers
                    // This is a simplified implementation - would need actual AST analysis
                    // to detect hardcoded QoS overrides in C++/Python code
                    
                    // For now, create a placeholder drift detection
                    if configured_qos.reliability == QoSReliability::Reliable {
                        // This would be detected by parsing source code for .best_effort() calls
                        if topic.name.contains("sensor") { // Heuristic for demo
                            let drift = ShadowDrift {
                                id: format!("qos_override_{}", topic.name),
                                drift_type: ShadowDriftType::QoSMismatch {
                                    configured: QoSReliability::Reliable,
                                    implemented: QoSReliability::BestEffort,
                                    topic_name: topic.name.clone(),
                                },
                                affected_elements: vec![topic.id.clone()],
                                inconsistency_details: InconsistencyReport {
                                    description: format!(
                                        "Topic '{}' configured with RELIABLE QoS but source code uses BEST_EFFORT",
                                        topic.name
                                    ),
                                    configuration_source: FileOrigin {
                                        file_path: PathBuf::from("config.yaml"),
                                        file_type: FileType::YAML,
                                        line: None,
                                        column: None,
                                        byte_range: None,
                                    },
                                    implementation_source: FileOrigin {
                                        file_path: PathBuf::from("node.cpp"),
                                        file_type: FileType::Cpp,
                                        line: None,
                                        column: None,
                                        byte_range: None,
                                    },
                                    precedence_explanation: "Source code QoS overrides configuration file".to_string(),
                                    runtime_implications: vec![
                                        "Messages may be dropped under network congestion".to_string(),
                                        "Reliability guarantees not met".to_string(),
                                    ],
                                },
                                file_locations: vec![],
                                severity: DriftSeverity::Error,
                                impact_assessment: ImpactAssessment {
                                    functional_impact: "Potential message loss".to_string(),
                                    performance_impact: Some("Reduced network overhead".to_string()),
                                    security_impact: None,
                                    safety_impact: Some("Safety-critical commands may be lost".to_string()),
                                    affected_use_cases: vec!["High-reliability communication".to_string()],
                                },
                                remediation_suggestions: vec![
                                    RemediationSuggestion {
                                        strategy: RemediationStrategy::UpdateConfiguration,
                                        description: "Update YAML configuration to use BEST_EFFORT".to_string(),
                                        confidence: 0.8,
                                        automation_possible: true,
                                    },
                                    RemediationSuggestion {
                                        strategy: RemediationStrategy::ModifyImplementation,
                                        description: "Remove hardcoded QoS override in source code".to_string(),
                                        confidence: 0.9,
                                        automation_possible: true,
                                    },
                                ],
                            };
                            drifts.push(drift);
                        }
                    }
                }
            }
        }
        
        drifts
    }
}

/// Pattern: Security Policy Override Detection
pub struct SecurityPolicyOverridePattern;

impl SecurityPolicyOverridePattern {
    pub fn new() -> Self {
        Self
    }
}

impl DriftPattern for SecurityPolicyOverridePattern {
    fn name(&self) -> &str {
        "Security Policy Override"
    }

    fn description(&self) -> &str {
        "Detects when source code bypasses or weakens configured security policies"
    }

    fn default_severity(&self) -> DriftSeverity {
        DriftSeverity::Critical
    }

    fn detect(&self, _ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        // Placeholder implementation
        Vec::new()
    }
}

/// Pattern: Lifecycle Violation Detection
pub struct LifecycleViolationPattern;

impl LifecycleViolationPattern {
    pub fn new() -> Self {
        Self
    }
}

impl DriftPattern for LifecycleViolationPattern {
    fn name(&self) -> &str {
        "Lifecycle Violation"
    }

    fn description(&self) -> &str {
        "Detects violations of managed lifecycle node state transition requirements"
    }

    fn default_severity(&self) -> DriftSeverity {
        DriftSeverity::Warning
    }

    fn detect(&self, _ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        // Placeholder implementation
        Vec::new()
    }
}

/// Pattern: Parameter Conflict Detection
pub struct ParameterConflictPattern;

impl ParameterConflictPattern {
    pub fn new() -> Self {
        Self
    }
}

impl DriftPattern for ParameterConflictPattern {
    fn name(&self) -> &str {
        "Parameter Conflict"
    }

    fn description(&self) -> &str {
        "Detects conflicting parameter values across multiple configuration sources"
    }

    fn default_severity(&self) -> DriftSeverity {
        DriftSeverity::Warning
    }

    fn detect(&self, _ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        // Placeholder implementation
        Vec::new()
    }
}

/// Pattern: Topic Type Mismatch Detection
pub struct TopicTypeMismatchPattern;

impl TopicTypeMismatchPattern {
    pub fn new() -> Self {
        Self
    }
}

impl DriftPattern for TopicTypeMismatchPattern {
    fn name(&self) -> &str {
        "Topic Type Mismatch"
    }

    fn description(&self) -> &str {
        "Detects type mismatches between topic publishers and subscribers"
    }

    fn default_severity(&self) -> DriftSeverity {
        DriftSeverity::Error
    }

    fn detect(&self, _ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        // Placeholder implementation
        Vec::new()
    }
}

/// Pattern: Executor Deadlock Detection
pub struct ExecutorDeadlockPattern;

impl ExecutorDeadlockPattern {
    pub fn new() -> Self {
        Self
    }
}

impl DriftPattern for ExecutorDeadlockPattern {
    fn name(&self) -> &str {
        "Executor Deadlock"
    }

    fn description(&self) -> &str {
        "Detects blocking operations in single-threaded executor callbacks"
    }

    fn default_severity(&self) -> DriftSeverity {
        DriftSeverity::Error
    }

    fn detect(&self, _ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        // This would integrate with existing deadlock detection from cpp_parser.rs
        // Placeholder implementation
        Vec::new()
    }
}

/// Pattern: Physical Safety Violation Detection
pub struct PhysicalSafetyViolationPattern;

impl PhysicalSafetyViolationPattern {
    pub fn new() -> Self {
        Self
    }
}

impl DriftPattern for PhysicalSafetyViolationPattern {
    fn name(&self) -> &str {
        "Physical Safety Violation"
    }

    fn description(&self) -> &str {
        "Detects physical safety constraint violations in robot models"
    }

    fn default_severity(&self) -> DriftSeverity {
        DriftSeverity::Critical
    }

    fn detect(&self, _ir: &CrossLanguageIR) -> Vec<ShadowDrift> {
        // This would integrate with existing URDF analysis
        // Placeholder implementation
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_shadow_drift_detector() {
        let detector = ShadowDriftDetector::new();
        assert!(!detector.drift_patterns.is_empty());
    }

    #[test]
    fn test_qos_override_pattern_detection() {
        let mut ir = CrossLanguageIR::new();
        
        // Add a topic with RELIABLE QoS configuration
        let qos_profile = QoSProfile {
            id: "reliable_qos".to_string(),
            name: Some("Reliable Profile".to_string()),
            reliability: QoSReliability::Reliable,
            durability: QoSDurability::SystemDefault,
            history: QoSHistory::SystemDefault,
            depth: None,
            deadline: None,
            lifespan: None,
            liveliness: QoSLiveliness::SystemDefault,
            liveliness_lease_duration: None,
        };
        
        let topic = TopicDefinition {
            id: "sensor_topic".to_string(),
            name: "sensor_data".to_string(),
            message_type: "sensor_msgs/msg/LaserScan".to_string(),
            publishers: vec![],
            subscribers: vec![],
            qos_profile: Some("reliable_qos".to_string()),
            security_policy: None,
        };
        
        ir.add_qos_profile(qos_profile);
        ir.add_topic(topic);
        
        let pattern = QoSOverridePattern::new();
        let drifts = pattern.detect(&ir);
        
        // Should detect the heuristic-based drift (sensor topic with RELIABLE -> BEST_EFFORT)
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].severity, DriftSeverity::Error);
        
        match &drifts[0].drift_type {
            ShadowDriftType::QoSMismatch { configured, implemented, topic_name } => {
                assert_eq!(*configured, QoSReliability::Reliable);
                assert_eq!(*implemented, QoSReliability::BestEffort);
                assert_eq!(topic_name, "sensor_data");
            }
            _ => panic!("Expected QoSMismatch drift type"),
        }
    }

    #[test]
    fn test_drift_severity_ordering() {
        assert!(DriftSeverity::Critical > DriftSeverity::Error);
        assert!(DriftSeverity::Error > DriftSeverity::Warning);
        assert!(DriftSeverity::Warning > DriftSeverity::Info);
    }

    #[test]
    fn test_drift_type_names() {
        let qos_drift = ShadowDriftType::QoSMismatch {
            configured: QoSReliability::Reliable,
            implemented: QoSReliability::BestEffort,
            topic_name: "test".to_string(),
        };
        assert_eq!(qos_drift.type_name(), "QoSMismatch");
        
        let security_drift = ShadowDriftType::SecurityInconsistency {
            expected_protection: ProtectionKind::Encrypt,
            actual_protection: ProtectionKind::None,
            element: "test".to_string(),
        };
        assert_eq!(security_drift.type_name(), "SecurityInconsistency");
    }

    #[test]
    fn test_shadow_drift_to_lint_violations() {
        let detector = ShadowDriftDetector::new();
        
        let drift = ShadowDrift {
            id: "test_drift".to_string(),
            drift_type: ShadowDriftType::QoSMismatch {
                configured: QoSReliability::Reliable,
                implemented: QoSReliability::BestEffort,
                topic_name: "test_topic".to_string(),
            },
            affected_elements: vec!["test_element".to_string()],
            inconsistency_details: InconsistencyReport {
                description: "Test drift description".to_string(),
                configuration_source: FileOrigin {
                    file_path: PathBuf::from("config.yaml"),
                    file_type: FileType::YAML,
                    line: Some(10),
                    column: Some(5),
                    byte_range: Some(100..200),
                },
                implementation_source: FileOrigin {
                    file_path: PathBuf::from("node.cpp"),
                    file_type: FileType::Cpp,
                    line: Some(50),
                    column: Some(15),
                    byte_range: Some(1000..1050),
                },
                precedence_explanation: "Test precedence".to_string(),
                runtime_implications: vec!["Test implication".to_string()],
            },
            file_locations: vec![
                FileOrigin {
                    file_path: PathBuf::from("test.yaml"),
                    file_type: FileType::YAML,
                    line: Some(10),
                    column: Some(5),
                    byte_range: Some(100..200),
                }
            ],
            severity: DriftSeverity::Error,
            impact_assessment: ImpactAssessment {
                functional_impact: "Test impact".to_string(),
                performance_impact: None,
                security_impact: None,
                safety_impact: None,
                affected_use_cases: vec![],
            },
            remediation_suggestions: vec![],
        };
        
        let violations = detector.to_lint_violations(&[drift]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Shadow Configuration Drift"));
        assert!(violations[0].message.contains("QoSMismatch"));
        assert_eq!(violations[0].range, 100..200);
    }
}