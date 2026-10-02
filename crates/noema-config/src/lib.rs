use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::env::{
    ENV_KEY_API_ENDPOINT, ENV_KEY_API_KEY, ENV_KEY_API_MODEL, ENV_KEY_ROOT_WORKDIR, env_get_optional, env_get_required};

mod env;

pub const NOEMA_CONFIG_FILE: &str = "config.json";

    
pub fn noema_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".noema"))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    pub model_name: String,
    pub api_key: Option<String>,
    pub endpoint: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NoemaConfig {
    pub llm: LlmConfig,
    pub workdir: PathBuf,
}

impl NoemaConfig {
    pub fn example() -> String {
        let example = NoemaConfig {
            llm: LlmConfig {
                model_name: "gpt-4o".to_string(),
                api_key: Some("sk-...".to_string()),
                endpoint: "https://api.openai.com/v1".to_string(),
            },
            workdir: PathBuf::from("/path/to/workdir"),
        };
        serde_json::to_string_pretty(&example).unwrap()
    }
    pub fn from_env() -> Result<Self, anyhow::Error> {
        let llm_config = LlmConfig {
            model_name: env_get_required(ENV_KEY_API_MODEL)?,
            api_key: env_get_optional(ENV_KEY_API_KEY),
            endpoint: env_get_required(ENV_KEY_API_ENDPOINT)?,
        };

        Ok(NoemaConfig {
            llm: llm_config,
            workdir: PathBuf::from(env_get_required(ENV_KEY_ROOT_WORKDIR).expect("unable to get workdir")),
        })
    }

    pub fn from_home() -> Result<Self, anyhow::Error> {
        let noema_path = noema_path().expect("unable to read noema path from home");
        std::fs::create_dir_all(&noema_path)?;

        let config_path = noema_path.join(NOEMA_CONFIG_FILE);
        let contents = std::fs::read_to_string(&config_path).with_context(|| {
            format!(
                "unable to read config file from {}\nexpected format:\n{}",
                config_path.display(),
                Self::example()
            )
        })?;
        let config: NoemaConfig = serde_json::from_str(&contents).with_context(|| {
            format!(
                "unable to parse config file {}\nexpected format:\n{}",
                config_path.display(),
                Self::example()
            )
        })?;
        Ok(config)
    }
}
