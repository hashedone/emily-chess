use std::collections::HashMap;
use std::time::Duration;

use derivative::Derivative;
use serde::Deserialize;

use crate::adapters::debug::FlatOptExt;

/// General configuration (config.toml schema)
#[derive(Derivative, Deserialize, Default)]
#[derivative(Debug)]
pub struct Config {
    /// Engine configuration
    #[derivative(Debug(format_with = "FlatOptExt::fmt"))]
    pub engine: Option<Engine>,
    /// Game review configuration
    #[serde(default)]
    pub rev: Rev,
    /// Logging configuration
    #[serde(default)]
    pub logging: Logging,
}

/// Cross-functionality engine configuration
#[derive(Derivative, Deserialize)]
#[derivative(Debug)]
pub struct Engine {
    /// Engine name for debugging and caching
    pub name: String,
    /// Command to run the engine
    pub command: String,
    /// Arguments to pass to the engine
    #[serde(default)]
    pub args: Vec<String>,
    /// Path where the engine should be executed
    #[derivative(Debug(format_with = "FlatOptExt::fmt"))]
    pub pwd: Option<String>,
    /// Engine options set on startup
    #[serde(default)]
    pub options: HashMap<String, String>,
    /// Debug mode (all debug information would be forwarded to the log)
    #[serde(default)]
    pub debug: bool,
}

/// Game review configuration
#[derive(Derivative, Deserialize, Default)]
#[derivative(Debug)]
pub struct Rev {
    /// Analysis depth limit (per move)
    #[derivative(Debug(format_with = "FlatOptExt::fmt"))]
    pub depth: Option<u8>,
    /// Analysis time limit (per move)
    #[derivative(Debug(format_with = "FlatOptExt::fmt"))]
    pub time: Option<Duration>,
}

#[derive(Deserialize, Default, Debug)]
pub struct Logging {
    /// Filter directives to attach
    pub filter: Vec<String>,
}
