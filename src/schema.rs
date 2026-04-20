use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Person,
    Career,
    Technical,
    Infrastructure,
    ProjectMemory,
    Vibe,
    Project,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Person => "person",
            Category::Career => "career",
            Category::Technical => "technical",
            Category::Infrastructure => "infrastructure",
            Category::ProjectMemory => "project-memory",
            Category::Vibe => "vibe",
            Category::Project => "project",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Wing {
    Projects,
    Infrastructure,
    Nexpublica,
    Personal,
    Career,
    Vibe,
}

impl Wing {
    pub fn as_str(&self) -> &'static str {
        match self {
            Wing::Projects => "projects",
            Wing::Infrastructure => "infrastructure",
            Wing::Nexpublica => "nexpublica",
            Wing::Personal => "personal",
            Wing::Career => "career",
            Wing::Vibe => "vibe",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Hall {
    Facts,
    Events,
    Decisions,
    Discoveries,
    Preferences,
}

impl Hall {
    pub fn as_str(&self) -> &'static str {
        match self {
            Hall::Facts => "facts",
            Hall::Events => "events",
            Hall::Decisions => "decisions",
            Hall::Discoveries => "discoveries",
            Hall::Preferences => "preferences",
        }
    }
}

/// The payload we write to Qdrant. Mirrors the palace schema established 2026-04-19.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    pub category: String,
    pub wing: String,
    pub room: String,
    pub hall: String,
    pub text: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}

/// A point as returned to MCP callers — the structured view of a palace memory.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Memory {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    pub text: String,
    pub category: String,
    pub wing: String,
    pub room: String,
    pub hall: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}
