pub mod blocklist;
pub mod staleness;

pub use blocklist::{Blocklist, BLOCKLIST_PATTERNS};
pub use staleness::{Confidence, StalenessChecker, StalenessResult};

use serde::{Deserialize, Serialize};
use crate::detector::types::{DetectedProject, SafetyTier};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCategory {
    QuickWin,
    GhostCleanup,
    DeepReview,
    Keep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedProject {
    pub root_path: String,
    pub category: ActionCategory,
    pub total_size_bytes: u64,
    pub staleness_score: f32, // 0.0 (fresh) to 1.0 (very stale)
    pub recommendation: String,
}

pub struct RuleEngine;

impl RuleEngine {
    pub fn classify(project: &DetectedProject) -> ClassifiedProject {
        let total_size = project.total_artifact_size_bytes();
        
        // Use the new StalenessChecker (placeholder threshold: 90 days)
        let git_cache = crate::cleaner::git::GitSessionCache::new();
        let checker = StalenessChecker::new(90, git_cache);
        let staleness_res = checker.evaluate(&project.root_path, Some(project.signature_mtime)).ok();
        
        let staleness = staleness_res.as_ref().map(|r| if r.is_stale { 1.0 } else { 0.1 }).unwrap_or(0.0);
        
        let mut category = ActionCategory::DeepReview;
        let mut recommendation = "Review internal artifacts before cleaning.".to_string();

        let has_ghosts = project.artifacts.iter().any(|a| a.is_ghost);
        let all_safe = project.artifacts.iter().all(|a| a.safety_tier == SafetyTier::Safe);
        let has_blocked = project.artifacts.iter().any(|a| a.safety_tier == SafetyTier::Blocked);

        if has_blocked {
            category = ActionCategory::Keep;
            recommendation = "Contains protected system files. Not recommended for deletion.".to_string();
        } else if all_safe && total_size > 100 * 1024 * 1024 && staleness > 0.5 {
            category = ActionCategory::QuickWin;
            recommendation = format!("Safe to delete. Reclaims {} MB.", total_size / (1024 * 1024));
        } else if has_ghosts {
            category = ActionCategory::GhostCleanup;
            recommendation = "Historical traces found. Safe to remove from logs/history.".to_string();
        }

        ClassifiedProject {
            root_path: project.root_path.to_string_lossy().to_string(),
            category,
            total_size_bytes: total_size,
            staleness_score: staleness,
            recommendation,
        }
    }
}

