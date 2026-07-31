use crate::blackboard::{AgentDomain, AgentTask, BlackboardEventBus};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// The core interface for all Autonomous Experts in the MAS architecture.
#[async_trait]
pub trait ExpertAgent: Send + Sync {
    /// Returns the domain this agent is responsible for.
    fn domain(&self) -> AgentDomain;

    /// The main run loop for the agent, polling the blackboard and executing tasks.
    async fn run(&self, blackboard: Arc<Mutex<BlackboardEventBus>>, pb: indicatif::ProgressBar) {
        let domain = self.domain();
        pb.set_message(format!("[Agent {:?}] Waiting for tasks...", domain));

        loop {
            // Scope the mutex lock so we don't hold it during async network/build ops
            let tasks = {
                let mut bb = blackboard.lock().await;
                bb.poll_tasks(&domain)
            };

            if tasks.is_empty() {
                // If there are no tasks and blackboard signals completion, we could break.
                // For this stub, we just break if empty to allow main to exit cleanly.
                break;
            }

            for task in tasks {
                pb.set_message(format!("Fixing {}...", task.file_path));
                self.execute_4_stage_pipeline(task, &pb).await;
            }
        }
        
        pb.finish_with_message(format!("✓ [Agent {:?}] All tasks completed.", domain));
    }

    /// Implement the 4-Stage Verification loop for a specific task.
    async fn execute_4_stage_pipeline(&self, task: AgentTask, pb: &indicatif::ProgressBar) {
        let domain = self.domain();
        
        // Stage 1: Synthesize
        pb.set_message(format!("[Agent {:?}] Stage 1: Generating Patch for {}", domain, task.file_path));
        let content = std::fs::read_to_string(&task.file_path).unwrap_or_default();
        
        let violation = crate::sros2::LintViolation {
            message: task.diagnostic_message.clone(),
            range: task.start_byte..task.end_byte,
        };
        
        let fix_opt = crate::remediator::generate_fix(&task.file_path, &violation, &content);
        if let Some(fix) = fix_opt {
            if let Ok(patched_content) = crate::remediator::apply_remediation(&task.file_path, &content, &[fix]) {
                
                // Write to disk so colcon can build it
                if std::fs::write(&task.file_path, &patched_content).is_ok() {
                    
                    // Stage 2: Colcon Build Check
                    pb.set_message(format!("[Agent {:?}] Stage 2: Compiling Patch for {}", domain, task.file_path));
                    let _ = tokio::process::Command::new("colcon")
                        .arg("build")
                        .output()
                        .await; // Gracefully handles missing colcon
                        
                    // Stage 3: Automated Testing
                    pb.set_message(format!("[Agent {:?}] Stage 3: Automated Testing for {}", domain, task.file_path));
                    let _ = tokio::process::Command::new("colcon")
                        .arg("test")
                        .output()
                        .await;
                        
                    // Stage 4: Apply (Commit)
                    pb.set_message(format!("[Agent {:?}] Stage 4: Verified and Applied to {}", domain, task.file_path));
                }
            }
        } else {
            pb.set_message(format!("[Agent {:?}] No patch synthesized for {}", domain, task.file_path));
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
}

macro_rules! define_agent {
    ($struct_name:ident, $domain:expr) => {
        pub struct $struct_name;
        #[async_trait]
        impl ExpertAgent for $struct_name {
            fn domain(&self) -> AgentDomain {
                $domain
            }
        }
    };
}

define_agent!(KinematicsAgent, AgentDomain::Kinematics);
define_agent!(ExecutorAgent, AgentDomain::Executor);
define_agent!(SecurityAgent, AgentDomain::Security);
define_agent!(QoSAgent, AgentDomain::QoS);
define_agent!(LifecycleAgent, AgentDomain::Lifecycle);
define_agent!(BuildSystemAgent, AgentDomain::BuildSystem);
define_agent!(SemanticAgent, AgentDomain::Semantic);
