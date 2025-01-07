use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

/// General configuration (config.toml schema)
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    /// Engine configuration
    pub engine: Option<Engine>,
    /// Game review configuration
    #[serde(default)]
    pub rev: Rev,
}

/// Cross-functionality engine configuration
#[derive(Debug, Deserialize)]
pub struct Engine {
    /// Engine name for debugging and caching
    pub name: String,
    /// Command to run the engine
    pub command: String,
    /// Arguments to pass to the engine
    #[serde(default)]
    pub args: Vec<String>,
    /// Path where the engine should be executed
    pub pwd: Option<String>,
    /// Engine options set on startup
    #[serde(default)]
    pub options: HashMap<String, String>,
    /// Debug mode (all debug information would be forwarded to the log)
    #[serde(default)]
    pub debug: bool,
}

/// Game review configuration
#[derive(Debug, Deserialize, Default)]
pub struct Rev {
    /// Analysis depth limit (per move)
    pub depth: Option<u8>,
    /// Analysis time limit (per move)
    pub time: Option<Duration>,
}
