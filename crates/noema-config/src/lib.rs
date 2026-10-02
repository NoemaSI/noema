use std::path::PathBuf;

use crate::env::{
    ENV_KEY_API_ENDPOINT, ENV_KEY_API_KEY, ENV_KEY_API_MODEL, ENV_KEY_ROOT_WORKDIR, env_get_optional, env_get_required};

mod env;

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub model_name: String,
    pub api_key: Option<String>,
    pub endpoint: String,
}

#[derive(Debug, Clone)]
pub struct NoemaConfig {
    pub llm: LlmConfig,
    pub workdir: PathBuf,
}

impl NoemaConfig {
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
}
