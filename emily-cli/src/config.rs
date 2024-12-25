use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub engine: Option<Engine>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Engine {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub pwd: Option<String>,
}
