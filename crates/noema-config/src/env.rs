use std::{env::{self, VarError}};

pub const ENV_KEY_API_MODEL: &str = "API_MODEL";
pub const ENV_KEY_API_KEY: &str = "API_KEY";
pub const ENV_KEY_API_ENDPOINT: &str = "API_ENDPOINT";
pub const ENV_KEY_ROOT_WORKDIR: &str = "ROOT_WORKDIR";

pub fn env_get_required(key: &str) -> Result<String, VarError> {
    env::var(key)
}  

pub fn env_get_optional(key: &str) -> Option<String> {
    match env::var(key) {
        Ok(val) => Some(val),
        Err(_) => None
    }
}  


