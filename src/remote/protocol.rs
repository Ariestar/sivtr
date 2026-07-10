use serde::{Deserialize, Serialize};
use sivtr_core::record::{WorkRecord, WorkRef};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    pub name: String,
    pub version: String,
    pub protocol: String,
    pub capabilities: Vec<String>,
}

impl AgentCard {
    pub fn current() -> Self {
        Self {
            name: "sivtr".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: "sivtr/1".to_string(),
            capabilities: vec![
                "source".to_string(),
                "resolve".to_string(),
                "resolve-part".to_string(),
                "search".to_string(),
                "sessions".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRequest {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceResponse {
    pub records: Vec<WorkRecord>,
    pub anchors: Vec<WorkRef>,
}
