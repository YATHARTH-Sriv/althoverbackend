use std::{env, sync::Arc};

const DEVELOPMENT_TOKEN_PEPPER: &str = "hover-local-dev-pepper";

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub frontend_url: String,
    pub node_internal_url: String,
    pub internal_service_token: Option<String>,
    pub extension_ids: Arc<[String]>,
    pub extension_token_pepper: Option<String>,
    pub is_production: bool,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let is_production = env::var("NODE_ENV").as_deref() == Ok("production");
        let extension_token_pepper = non_empty_env("EXTENSION_TOKEN_PEPPER")
            .or_else(|| (!is_production).then(|| DEVELOPMENT_TOKEN_PEPPER.to_owned()));

        Self {
            frontend_url: env::var("FRONTEND_URL")
                .unwrap_or_else(|_| "http://localhost:3000".to_owned())
                .trim_end_matches('/')
                .to_owned(),
            node_internal_url: env::var("NODE_INTERNAL_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8000".to_owned())
                .trim_end_matches('/')
                .to_owned(),
            internal_service_token: non_empty_env("INTERNAL_SERVICE_TOKEN"),
            extension_ids: env::var("EXTENSION_IDS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .into(),
            extension_token_pepper,
            is_production,
        }
    }

    pub fn for_tests() -> Self {
        Self {
            frontend_url: "http://localhost:3000".to_owned(),
            node_internal_url: "http://127.0.0.1:8000".to_owned(),
            internal_service_token: Some("test-internal-token".to_owned()),
            extension_ids: Arc::from([]),
            extension_token_pepper: Some("test-extension-pepper".to_owned()),
            is_production: false,
        }
    }

    pub fn node_url(&self, path: &str) -> String {
        format!("{}{}", self.node_internal_url, path)
    }

    pub fn frontend_url(&self, path: &str) -> String {
        format!("{}{}", self.frontend_url, path)
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
