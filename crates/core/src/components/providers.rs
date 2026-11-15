use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    OneClick,
    #[serde(rename = "oauth_app")]
    OAuthApp,
    ApiKey,
    Open,
    Local,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub slug: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub url: String,
    pub docs: String,
    pub icon: String,
    pub methods: Vec<Method>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<StaticOAuth>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StaticOAuth {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub scopes: Vec<String>,
}
#[derive(Debug, thiserror::Error)]
#[error("embedded integration registry is invalid")]
pub struct Error;
pub fn registry() -> Result<Vec<Entry>, Error> {
    serde_json::from_str(include_str!("../../integrations.json")).map_err(|_| Error)
}
