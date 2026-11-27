use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode, Url};
use rmcp::{ServiceExt, transport::stdio};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::{collections::HashSet, env, fmt, time::Duration};
use uuid::Uuid;

const MAX_CONCURRENT_TOOL_OPERATIONS: usize = 8;
const MAX_CONCURRENT_WAIT_OPERATIONS: usize = 6;
const MAX_CONCURRENT_CONTROL_OPERATIONS: usize = 2;
const MAX_CONTROL_QUEUE_WAIT: Duration = Duration::from_secs(5);
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
// Two maximum outputs can expand sixfold when JSON escapes control characters.
const MAX_EXECUTION_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_METADATA_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_ENVIRONMENT_LIST_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_USAGE_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_USAGE_CURSOR_BYTES: usize = 512;
const MAX_ERROR_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_PYTHON_REQUIREMENTS: usize = 32;
const MAX_PYTHON_REQUIREMENT_BYTES: usize = 200;
const MAX_PYTHON_BUILD_REQUEST_BYTES: usize = 128 * 1024;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 10_000;
const MAX_WAIT_TIMEOUT_MS: u64 = 30_000;
const MIN_POLL_INTERVAL_MS: u64 = 100;
const MAX_POLL_INTERVAL_MS: u64 = 5_000;

mod tools;
pub use tools::MpakaMcpServer;
#[cfg(test)]
use tools::*;

#[derive(Clone)]
pub struct McpConfig {
    pub api_base_url: Url,
    pub api_key: String,
    pub request_timeout: Duration,
}

impl fmt::Debug for McpConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpConfig")
            .field("api_base_url", &self.api_base_url)
            .field("api_key", &"[REDACTED]")
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl McpConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let base = resolve_api_base_url(
            env::var("MPAKA_API_URL").ok().as_deref(),
            env::var("MPAKA_API_BASE_URL").ok().as_deref(),
        );
        let api_base_url = Url::parse(&base).map_err(|_| ConfigError::InvalidApiUrl)?;
        let api_key = env::var("MPAKA_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .ok_or(ConfigError::MissingApiKey)?;
        let timeout_seconds = env::var("MPAKA_API_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(15);
        Self::new(api_base_url, api_key, Duration::from_secs(timeout_seconds))
    }

    pub fn new(
        api_base_url: Url,
        api_key: String,
        request_timeout: Duration,
    ) -> Result<Self, ConfigError> {
        if !matches!(api_base_url.scheme(), "http" | "https")
            || api_base_url.host_str().is_none()
            || api_base_url.username() != ""
            || api_base_url.password().is_some()
            || api_base_url.query().is_some()
            || api_base_url.fragment().is_some()
        {
            return Err(ConfigError::InvalidApiUrl);
        }
        let is_loopback = api_base_url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        if api_base_url.scheme() != "https" && !is_loopback {
            return Err(ConfigError::InvalidApiUrl);
        }
        if api_key.trim().is_empty() {
            return Err(ConfigError::MissingApiKey);
        }
        if !(1..=120).contains(&request_timeout.as_secs()) {
            return Err(ConfigError::InvalidTimeout);
        }
        Ok(Self {
            api_base_url,
            api_key,
            request_timeout,
        })
    }
}

fn resolve_api_base_url(common: Option<&str>, legacy: Option<&str>) -> String {
    common
        .or(legacy)
        .unwrap_or("https://api.mpaka.dev")
        .to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidApiUrl,
    MissingApiKey,
    InvalidTimeout,
}

#[derive(Clone)]
pub struct ApiClient {
    config: McpConfig,
    http: Client,
}

impl ApiClient {
    pub fn new(config: McpConfig) -> Result<Self, reqwest::Error> {
        let http = Client::builder()
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { config, http })
    }

    async fn submit_execution(
        &self,
        code: String,
        idempotency_key: String,
        timeout_ms: Option<u64>,
        memory_profile: Option<String>,
        cpu_profile: Option<String>,
        environment_id: Option<Uuid>,
    ) -> Result<Execution, ApiError> {
        if code.trim().is_empty() {
            return Err(ApiError::input("code must not be empty"));
        }
        validate_idempotency_key(&idempotency_key)?;
        let mut body = json!({"language": "python", "code": code});
        if let Some(timeout_ms) = timeout_ms {
            body["timeout_ms"] = json!(timeout_ms);
        }
        if let Some(memory_profile) = memory_profile {
            body["memory_profile"] = json!(memory_profile);
        }
        if let Some(cpu_profile) = cpu_profile {
            body["cpu_profile"] = json!(cpu_profile);
        }
        if let Some(environment_id) = environment_id {
            body["environment_id"] = json!(environment_id);
        }

        let response = self
            .request(self.http.post(self.endpoint("/v1/executions")))
            .header("Idempotency-Key", idempotency_key)
            .json(&body)
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_response(response).await
    }

    async fn get_execution(&self, id: Uuid) -> Result<Execution, ApiError> {
        let response = self
            .request(
                self.http
                    .get(self.endpoint(&format!("/v1/executions/{id}"))),
            )
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_response(response).await
    }

    async fn cancel_execution(&self, id: Uuid) -> Result<Execution, ApiError> {
        let response = self
            .request(
                self.http
                    .post(self.endpoint(&format!("/v1/executions/{id}/cancel"))),
            )
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_response(response).await
    }

    async fn create_environment(
        &self,
        requirements: Vec<String>,
        idempotency_key: String,
    ) -> Result<PythonEnvironment, ApiError> {
        validate_idempotency_key(&idempotency_key)?;
        let requirements = validate_python_requirements(requirements)?;
        let response = self
            .request(self.http.post(self.endpoint("/v1/environments")))
            .header("Idempotency-Key", idempotency_key)
            .json(&json!({"requirements": requirements}))
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_environment_response(response).await
    }

    async fn get_environment(&self, id: Uuid) -> Result<PythonEnvironment, ApiError> {
        let response = self
            .request(
                self.http
                    .get(self.endpoint(&format!("/v1/environments/{id}"))),
            )
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_environment_response(response).await
    }

    async fn list_environments(
        &self,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<EnvironmentPage, ApiError> {
        let mut endpoint = self.endpoint("/v1/environments");
        {
            let mut pairs = endpoint.query_pairs_mut();
            if let Some(page) = page {
                pairs.append_pair("page", &page.to_string());
            }
            if let Some(page_size) = page_size {
                pairs.append_pair("page_size", &page_size.to_string());
            }
        }
        let response = self
            .request(self.http.get(endpoint))
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_json_response(response, MAX_ENVIRONMENT_LIST_RESPONSE_BYTES).await
    }

    async fn get_usage(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: Option<u16>,
        offset: Option<u32>,
        cursor: Option<String>,
    ) -> Result<TenantUsage, ApiError> {
        let now = Utc::now();
        let until = until.unwrap_or(now);
        let since = since.unwrap_or(until - chrono::Duration::days(30));
        let limit = limit.unwrap_or(100);
        if cursor.is_some() && offset.is_some_and(|offset| offset != 0) {
            return Err(ApiError::input(
                "usage cursor cannot be combined with a nonzero offset",
            ));
        }
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > MAX_USAGE_CURSOR_BYTES)
        {
            return Err(ApiError::input("usage cursor is too large"));
        }
        validate_usage_query(since, until, limit, now)?;
        let mut endpoint = self.endpoint("/v1/usage");
        {
            let mut pairs = endpoint.query_pairs_mut();
            pairs
                .append_pair("since", &since.to_rfc3339())
                .append_pair("until", &until.to_rfc3339())
                .append_pair("limit", &limit.to_string());
            if let Some(cursor) = cursor {
                pairs.append_pair("cursor", &cursor);
            } else {
                pairs.append_pair("offset", &offset.unwrap_or(0).to_string());
            }
        }
        let response = self
            .request(self.http.get(endpoint))
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_usage_response(response).await
    }

    async fn get_usage_summary(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        group_by: Option<String>,
    ) -> Result<TenantUsageSummary, ApiError> {
        let now = Utc::now();
        let until = until.unwrap_or(now);
        let since = since.unwrap_or(until - chrono::Duration::days(30));
        if since >= until
            || until > now
            || until - since > chrono::Duration::days(366)
            || group_by.as_deref().is_some_and(|value| value != "day")
        {
            return Err(ApiError::input(
                "usage period must be ordered, no longer than 366 days, end no later than now, and group_by must be day",
            ));
        }
        let mut endpoint = self.endpoint("/v1/usage/summary");
        {
            let mut pairs = endpoint.query_pairs_mut();
            pairs
                .append_pair("since", &since.to_rfc3339())
                .append_pair("until", &until.to_rfc3339());
            if let Some(group_by) = group_by {
                pairs.append_pair("group_by", &group_by);
            }
        }
        let response = self
            .request(self.http.get(endpoint))
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_json_response(response, MAX_USAGE_RESPONSE_BYTES).await
    }

    async fn get_limits(&self) -> Result<ResourceLimits, ApiError> {
        let response = self
            .request(self.http.get(self.endpoint("/v1/limits")))
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_limits_response(response).await
    }

    async fn get_quota(&self) -> Result<TenantQuota, ApiError> {
        let response = self
            .request(self.http.get(self.endpoint("/v1/quota")))
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_json_response(response, MAX_METADATA_RESPONSE_BYTES).await
    }

    async fn list_executions(
        &self,
        page: Option<u32>,
        page_size: Option<u32>,
        status: Option<&str>,
        language: Option<&str>,
        query: Option<&str>,
        created_on: Option<&str>,
    ) -> Result<ExecutionPage, ApiError> {
        let mut endpoint = self.endpoint("/v1/executions");
        {
            let mut pairs = endpoint.query_pairs_mut();
            if let Some(page) = page {
                pairs.append_pair("page", &page.to_string());
            }
            if let Some(page_size) = page_size {
                pairs.append_pair("page_size", &page_size.to_string());
            }
            if let Some(status) = status {
                pairs.append_pair("status", status);
            }
            if let Some(language) = language {
                pairs.append_pair("language", language);
            }
            if let Some(query) = query {
                pairs.append_pair("q", query);
            }
            if let Some(created_on) = created_on {
                pairs.append_pair("created_on", created_on);
            }
        }
        let response = self
            .request(self.http.get(endpoint))
            .send()
            .await
            .map_err(ApiError::transport)?;
        decode_json_response(response, MAX_METADATA_RESPONSE_BYTES).await
    }

    fn request(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request.bearer_auth(&self.config.api_key)
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut base = self.config.api_base_url.clone();
        let prefix = base.path().trim_end_matches('/');
        base.set_path(&format!("{prefix}{path}"));
        base
    }
}

fn validate_python_requirements(requirements: Vec<String>) -> Result<Vec<String>, ApiError> {
    if requirements.is_empty() || requirements.len() > MAX_PYTHON_REQUIREMENTS {
        return Err(ApiError::input(
            "requirements must contain 1 to 32 pinned package versions",
        ));
    }

    let mut normalized = Vec::with_capacity(requirements.len());
    let mut names = HashSet::with_capacity(requirements.len());
    for requirement in requirements {
        if requirement.len() > MAX_PYTHON_REQUIREMENT_BYTES {
            return Err(ApiError::input(
                "each requirement must be at most 200 bytes",
            ));
        }
        let (name, version) = requirement
            .split_once("==")
            .ok_or_else(|| ApiError::input("requirements must be pinned package versions"))?;
        if version.is_empty() || version.contains("==") || version.contains('*') {
            return Err(ApiError::input(
                "requirements must be pinned package versions",
            ));
        }
        let name = normalize_python_package_name(name)?;
        if !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!+_-".contains(&byte))
            || !version.bytes().any(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(ApiError::input("requirement version is invalid"));
        }
        if !names.insert(name.clone()) {
            return Err(ApiError::input("duplicate Python package requirement"));
        }
        normalized.push(format!("{name}=={version}"));
    }
    normalized.sort_unstable();
    let serialized_len = 2
        + normalized.iter().map(|item| item.len() + 2).sum::<usize>()
        + normalized.len().saturating_sub(1);
    if serialized_len > MAX_PYTHON_BUILD_REQUEST_BYTES {
        return Err(ApiError::input("serialized requirements exceed 128 KiB"));
    }
    Ok(normalized)
}

fn normalize_python_package_name(name: &str) -> Result<String, ApiError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ApiError::input("Python package name is invalid"));
    }
    let mut normalized = String::with_capacity(name.len());
    let mut previous_separator = false;
    for byte in name.bytes() {
        if matches!(byte, b'.' | b'_' | b'-') {
            if !previous_separator {
                normalized.push('-');
            }
            previous_separator = true;
        } else {
            normalized.push(char::from(byte.to_ascii_lowercase()));
            previous_separator = false;
        }
    }
    if normalized.starts_with('-') || normalized.ends_with('-') {
        return Err(ApiError::input("Python package name is invalid"));
    }
    Ok(normalized)
}

fn validate_idempotency_key(idempotency_key: &str) -> Result<(), ApiError> {
    if idempotency_key.trim().is_empty()
        || idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        || !idempotency_key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(ApiError::input(
            "idempotency_key must be 1 to 128 visible ASCII characters",
        ));
    }
    Ok(())
}

fn parse_usage_timestamp(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|timestamp| timestamp.with_timezone(&Utc))
                .map_err(|_| ApiError::input(&format!("{field} must be an RFC 3339 timestamp")))
        })
        .transpose()
}

fn validate_usage_query(
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    limit: u16,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    if since >= until
        || until > now
        || until - since > chrono::Duration::days(366)
        || !(1..=1000).contains(&limit)
    {
        return Err(ApiError::input(
            "usage period must be ordered, no longer than 366 days, end no later than now, and limit must be between 1 and 1000",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct Execution {
    id: Uuid,
    status: String,
    language: String,
    timeout_ms: u64,
    memory_mb: u64,
    vcpu_count: u8,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    mode: String,
    #[serde(default)]
    output_truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct PythonEnvironment {
    id: Uuid,
    status: String,
    requirements: Vec<String>,
    artifact_sha256: Option<String>,
    artifact_size_bytes: Option<i64>,
    resolved_digest: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct EnvironmentPage {
    items: Vec<PythonEnvironment>,
    page: u32,
    page_size: u32,
    total: u64,
    has_next: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct TenantUsage {
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    attempt_count: i64,
    measured_attempt_count: i64,
    provisioned_vcpu_ms: i64,
    provisioned_mib_ms: i64,
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
    items: Vec<TenantUsageItem>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct TenantUsageSummary {
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    attempt_count: i64,
    measured_attempt_count: i64,
    pending_attempt_count: i64,
    unmeasured_terminal_attempt_count: i64,
    provisioned_vcpu_ms: i64,
    provisioned_mib_ms: i64,
    daily: Vec<TenantUsageDay>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct TenantUsageDay {
    date: String,
    attempt_count: i64,
    measured_attempt_count: i64,
    pending_attempt_count: i64,
    unmeasured_terminal_attempt_count: i64,
    provisioned_vcpu_ms: i64,
    provisioned_mib_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct TenantQuota {
    used_units: i64,
    limit_units: i64,
    active_count: i64,
    active_limit: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ExecutionSummary {
    id: Uuid,
    status: String,
    language: String,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ExecutionPage {
    items: Vec<ExecutionSummary>,
    page: u32,
    page_size: u32,
    total: u64,
    has_next: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct WaitExecutionResult {
    execution: Execution,
    timed_out: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct WaitEnvironmentResult {
    environment: PythonEnvironment,
    timed_out: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct TenantUsageItem {
    attempt_id: Uuid,
    assignment_id: Uuid,
    execution_id: Uuid,
    attempt_state: String,
    execution_status: String,
    vcpu_count: i16,
    memory_mb: i64,
    assigned_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    duration_ms: Option<i64>,
    provisioned_vcpu_ms: Option<i64>,
    provisioned_mib_ms: Option<i64>,
    metering_contract_version: i16,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ResourceLimits {
    language: String,
    max_code_bytes: usize,
    min_timeout_ms: u64,
    max_timeout_ms: u64,
    default_timeout_ms: u64,
    min_memory_mb: u64,
    max_memory_mb: u64,
    default_memory_mb: u64,
    guest_memory_profiles: Vec<String>,
    guest_cpu_profiles: Vec<String>,
    min_vcpu_count: u8,
    max_vcpu_count: u8,
    default_vcpu_count: u8,
}

pub async fn serve_stdio(
    config: McpConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = ApiClient::new(config)?;
    let server = MpakaMcpServer::new(client).serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

#[derive(Debug)]
struct ApiError {
    code: String,
    message: String,
}

impl ApiError {
    fn input(message: &str) -> Self {
        Self {
            code: "invalid_input".into(),
            message: message.into(),
        }
    }

    fn transport(error: reqwest::Error) -> Self {
        let code = if error.is_timeout() {
            "upstream_timeout"
        } else {
            "upstream_unavailable"
        };
        Self {
            code: code.into(),
            message: "mPaka API request failed".into(),
        }
    }

    fn response_too_large() -> Self {
        Self {
            code: "upstream_response_too_large".into(),
            message: "mPaka API response exceeded the allowed size".into(),
        }
    }

    fn invalid_json() -> Self {
        Self {
            code: "upstream_invalid_response".into(),
            message: "mPaka API returned an invalid response".into(),
        }
    }

    fn wait_timeout() -> Self {
        Self {
            code: "upstream_timeout".into(),
            message: "mPaka API request did not return a resource state before the wait deadline"
                .into(),
        }
    }
}

fn is_terminal_execution_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "timed_out" | "cancelled")
}

fn is_terminal_environment_status(status: &str) -> bool {
    matches!(status, "ready" | "failed")
}

async fn decode_response(response: reqwest::Response) -> Result<Execution, ApiError> {
    decode_json_response(response, MAX_EXECUTION_RESPONSE_BYTES).await
}

async fn decode_environment_response(
    response: reqwest::Response,
) -> Result<PythonEnvironment, ApiError> {
    decode_json_response(response, MAX_METADATA_RESPONSE_BYTES).await
}

async fn decode_usage_response(response: reqwest::Response) -> Result<TenantUsage, ApiError> {
    decode_json_response(response, MAX_USAGE_RESPONSE_BYTES).await
}

async fn decode_limits_response(response: reqwest::Response) -> Result<ResourceLimits, ApiError> {
    decode_json_response(response, MAX_METADATA_RESPONSE_BYTES).await
}

async fn decode_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
    max_success_bytes: usize,
) -> Result<T, ApiError> {
    let status = response.status();
    let body = read_bounded_body(
        response,
        if status.is_success() {
            max_success_bytes
        } else {
            MAX_ERROR_RESPONSE_BYTES
        },
    )
    .await?;
    if status.is_success() {
        return serde_json::from_slice(&body).map_err(|_| ApiError::invalid_json());
    }
    let payload = serde_json::from_slice::<ApiErrorBody>(&body).ok();
    let api_code = payload
        .as_ref()
        .and_then(|body| body.error.as_ref())
        .and_then(|error| error.code.as_deref())
        .filter(|code| !code.trim().is_empty());
    let code = api_code.map(str::to_owned).unwrap_or_else(|| {
        match status {
            StatusCode::UNAUTHORIZED => "unauthorized",
            StatusCode::FORBIDDEN => "forbidden",
            StatusCode::NOT_FOUND => "not_found",
            StatusCode::TOO_MANY_REQUESTS => "rate_limited",
            StatusCode::CONFLICT => "conflict",
            StatusCode::BAD_REQUEST => "invalid_input",
            _ => "upstream_unavailable",
        }
        .to_owned()
    });
    let message = payload
        .and_then(|body| body.error)
        .and_then(|error| error.message)
        .unwrap_or_else(|| format!("mPaka API returned HTTP {}", status.as_u16()));
    Err(ApiError { code, message })
}

async fn read_bounded_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, ApiError> {
    if response
        .content_length()
        .is_some_and(|content_length| content_length > max_bytes as u64)
    {
        return Err(ApiError::response_too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(ApiError::transport)? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ApiError::response_too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct ApiErrorBody {
    error: Option<ApiErrorDetails>,
}

#[derive(Deserialize)]
struct ApiErrorDetails {
    code: Option<String>,
    message: Option<String>,
}

pub fn parse_execution_id(value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| "execution_id must be a UUID".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::Body,
        extract::State,
        http::HeaderMap,
        routing::{get, post},
    };
    use futures_util::stream;
    use rmcp::model::ContentBlock;
    use std::convert::Infallible;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn checked_in_openapi_covers_mcp_routes_states_and_errors() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../contracts/openapi.json"))
                .expect("OpenAPI fixture");
        let paths = &contract["paths"];
        let schemas = &contract["components"]["schemas"];

        assert_eq!(
            paths["/v1/environments"]["post"]["operationId"],
            "create_python_environment"
        );
        assert_eq!(
            paths["/v1/environments/{id}"]["get"]["operationId"],
            "get_python_environment"
        );
        assert_eq!(
            paths["/v1/executions"]["post"]["operationId"],
            "create_execution"
        );
        for (path, method, status) in [
            ("/v1/environments", "post", "202"),
            ("/v1/environments", "post", "400"),
            ("/v1/environments", "post", "503"),
            ("/v1/environments/{id}", "get", "404"),
            ("/v1/environments/{id}", "get", "503"),
            ("/v1/executions", "post", "404"),
            ("/v1/executions", "post", "409"),
        ] {
            assert!(
                paths[path][method]["responses"].get(status).is_some(),
                "OpenAPI fixture is missing {status} for {method} {path}"
            );
        }
        for state in ["queued", "building", "ready", "failed"] {
            assert!(
                schemas["EnvironmentStatusSchema"]["enum"]
                    .as_array()
                    .expect("environment status enum")
                    .contains(&serde_json::Value::String(state.into()))
            );
        }
    }

    #[tokio::test]
    async fn environment_api_errors_preserve_openapi_error_codes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route(
                "/v1/environments",
                post(|| async {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error":{"code":"invalid_requirements","message":"pin each package"}})),
                    )
                }),
            )
            .route(
                "/v1/environments/{id}",
                get(|| async {
                    (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error":{"code":"environment_not_found","message":"not found"}})),
                    )
                }),
            );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let create_error = client
            .create_environment(vec!["requests==2.32.3".into()], "env-invalid-1".into())
            .await
            .expect_err("API rejects invalid package");
        let get_error = client
            .get_environment(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
            .await
            .expect_err("API cannot find environment");
        task.abort();

        assert_eq!(create_error.code, "invalid_requirements");
        assert_eq!(get_error.code, "environment_not_found");
    }

    #[derive(Clone, Default)]
    struct CapturedRequest(Arc<Mutex<Option<(HeaderMap, serde_json::Value)>>>);

    #[derive(Clone, Default)]
    struct SequenceCounter(Arc<AtomicUsize>);

    async fn sequence_execution(State(calls): State<SequenceCounter>) -> Json<serde_json::Value> {
        let mut execution = execution_json();
        execution["status"] = if calls.0.fetch_add(1, Ordering::Relaxed) == 0 {
            json!("running")
        } else {
            json!("completed")
        };
        Json(execution)
    }

    async fn pending_execution() -> Json<serde_json::Value> {
        let mut execution = execution_json();
        execution["status"] = json!("running");
        Json(execution)
    }

    async fn sequence_environment(State(calls): State<SequenceCounter>) -> Json<serde_json::Value> {
        let mut environment = environment_json();
        environment["status"] = if calls.0.fetch_add(1, Ordering::Relaxed) == 0 {
            json!("building")
        } else {
            json!("ready")
        };
        Json(environment)
    }

    async fn pending_environment() -> Json<serde_json::Value> {
        let mut environment = environment_json();
        environment["status"] = json!("building");
        Json(environment)
    }

    async fn capture_submission(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, body));
        Json(execution_json())
    }

    async fn capture_environment_creation(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, body));
        Json(environment_json())
    }

    async fn capture_cancellation(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, json!({})));
        Json(execution_json())
    }

    async fn capture_limits(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, json!({})));
        Json(json!({
            "language": "python",
            "max_code_bytes": 262144,
            "min_timeout_ms": 100,
            "max_timeout_ms": 10000,
            "default_timeout_ms": 3000,
            "min_memory_mb": 32,
            "max_memory_mb": 512,
            "default_memory_mb": 128,
            "guest_memory_profiles": ["standard", "custom_memory"],
            "guest_cpu_profiles": ["standard", "custom_cpu"],
            "min_vcpu_count": 1,
            "max_vcpu_count": 2,
            "default_vcpu_count": 1
        }))
    }

    async fn capture_quota(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, json!({})));
        Json(json!({
            "used_units": 7,
            "limit_units": 1000,
            "active_count": 2,
            "active_limit": 8
        }))
    }

    async fn capture_execution_list(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
        axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, json!({"query": uri.query()})));
        Json(json!({
            "items": [{
                "id": "00000000-0000-0000-0000-000000000001",
                "status": "completed",
                "language": "python",
                "created_at": "2026-11-24T00:00:00Z",
                "finished_at": "2026-11-24T00:00:01Z"
            }],
            "page": 2,
            "page_size": 10,
            "total": 11,
            "has_next": true
        }))
    }

    async fn capture_environment_list(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
        axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, json!({"query": uri.query()})));
        Json(json!({
            "items": [environment_json()],
            "page": 2,
            "page_size": 10,
            "total": 11,
            "has_next": true
        }))
    }

    async fn capture_usage(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
        axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, json!({"query": uri.query()})));
        Json(json!({
            "since": "2026-10-25T00:00:00Z",
            "until": "2026-11-24T00:00:00Z",
            "attempt_count": 1,
            "measured_attempt_count": 1,
            "provisioned_vcpu_ms": 2000,
            "provisioned_mib_ms": 256000,
            "has_more": false,
            "items": [{
                "attempt_id": "00000000-0000-0000-0000-000000000003",
                "assignment_id": "00000000-0000-0000-0000-000000000004",
                "execution_id": "00000000-0000-0000-0000-000000000001",
                "host_id": "00000000-0000-0000-0000-000000000005",
                "attempt_state": "completed",
                "execution_status": "completed",
                "vcpu_count": 1,
                "memory_mb": 128,
                "assigned_at": "2026-11-01T00:00:00Z",
                "started_at": "2026-11-01T00:00:01Z",
                "finished_at": "2026-11-01T00:00:02Z",
                "duration_ms": 1000,
                "provisioned_vcpu_ms": 1000,
                "provisioned_mib_ms": 128000,
                "metering_contract_version": 1
            }]
        }))
    }

    async fn capture_usage_summary(
        State(captured): State<CapturedRequest>,
        headers: HeaderMap,
        axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    ) -> Json<serde_json::Value> {
        *captured.0.lock().expect("capture lock") = Some((headers, json!({"query": uri.query()})));
        Json(json!({
            "since": "2026-11-01T00:00:00Z",
            "until": "2026-11-24T00:00:00Z",
            "attempt_count": 3,
            "measured_attempt_count": 2,
            "pending_attempt_count": 1,
            "unmeasured_terminal_attempt_count": 0,
            "provisioned_vcpu_ms": 3000,
            "provisioned_mib_ms": 384000,
            "daily": [{
                "date": "2026-11-23",
                "attempt_count": 3,
                "measured_attempt_count": 2,
                "pending_attempt_count": 1,
                "unmeasured_terminal_attempt_count": 0,
                "provisioned_vcpu_ms": 3000,
                "provisioned_mib_ms": 384000
            }]
        }))
    }

    async fn get_environment_response() -> Json<serde_json::Value> {
        Json(environment_json())
    }

    fn environment_json() -> serde_json::Value {
        json!({
            "id": "00000000-0000-0000-0000-000000000002",
            "status": "queued",
            "requirements": ["requests==2.32.3"],
            "artifact_sha256": null,
            "artifact_size_bytes": null,
            "resolved_digest": null,
            "created_at": "2026-11-24T00:00:00Z",
            "updated_at": "2026-11-24T00:00:00Z"
        })
    }

    fn execution_json() -> serde_json::Value {
        json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "status": "queued",
            "language": "python",
            "timeout_ms": 3000,
            "memory_mb": 512,
            "vcpu_count": 2,
            "created_at": "2026-11-24T00:00:00Z",
            "finished_at": null,
            "stdout": "",
            "stderr": "",
            "exit_code": null,
            "mode": "firecracker"
        })
    }

    #[test]
    fn allows_http_only_for_loopback_api_origins() {
        let key = "test-tenant-api-key".to_owned();
        assert!(
            McpConfig::new(
                Url::parse("http://127.0.0.1:5300").expect("loopback URL"),
                key.clone(),
                Duration::from_secs(15),
            )
            .is_ok()
        );
        assert!(matches!(
            McpConfig::new(
                Url::parse("http://api.example.test").expect("API URL"),
                key,
                Duration::from_secs(15),
            ),
            Err(ConfigError::InvalidApiUrl)
        ));
    }

    #[test]
    fn defaults_to_production_url_and_prefers_common_endpoint_variable() {
        assert_eq!(resolve_api_base_url(None, None), "https://api.mpaka.dev");
        assert_eq!(
            resolve_api_base_url(
                Some("https://common.example"),
                Some("https://legacy.example")
            ),
            "https://common.example"
        );
        assert_eq!(
            resolve_api_base_url(None, Some("https://legacy.example")),
            "https://legacy.example"
        );
    }

    #[test]
    fn debug_redacts_api_key() {
        let config = McpConfig::new(
            Url::parse("https://api.example.test").expect("API URL"),
            "secret-api-key-123".into(),
            Duration::from_secs(15),
        )
        .expect("valid MCP config");

        let debug = format!("{config:?}");
        assert!(!debug.contains("secret-api-key-123"));
        assert!(debug.contains("REDACTED"));
        assert!(debug.contains("api.example.test"));
    }

    #[tokio::test]
    async fn resource_profiles_come_from_authenticated_api_limits() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/limits", get(capture_limits))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let limits = client.get_limits().await.expect("resource limits");
        let result = MpakaMcpServer::new(client)
            .get_resource_profiles()
            .await
            .expect("resource profile tool result");
        task.abort();

        let (headers, _) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        assert_eq!(limits.guest_memory_profiles, ["standard", "custom_memory"]);
        assert_eq!(limits.guest_cpu_profiles, ["standard", "custom_cpu"]);
        assert_eq!(
            result.structured_content.as_ref().unwrap()["guest_memory_profiles"][1],
            "custom_memory"
        );
        let text = match &result.content[0] {
            ContentBlock::Text(text) => text.text.as_str(),
            _ => panic!("expected text result"),
        };
        assert!(text.contains("custom_memory"));
        assert!(text.contains("custom_cpu"));
    }

    #[tokio::test]
    async fn rejects_chunked_success_response_over_execution_body_limit() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new().route(
            "/v1/executions/{id}",
            get(|| async {
                let chunks = vec![
                    Ok::<_, Infallible>(" ".repeat(MAX_EXECUTION_RESPONSE_BYTES / 2)),
                    Ok::<_, Infallible>(" ".repeat(MAX_EXECUTION_RESPONSE_BYTES / 2 + 1)),
                ];
                (
                    [("content-type", "application/json")],
                    Body::from_stream(stream::iter(chunks)),
                )
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");
        let id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

        let error = client.get_execution(id).await.expect_err("body over limit");
        task.abort();

        assert_eq!(error.code, "upstream_response_too_large");
    }

    #[tokio::test]
    async fn accepts_maximum_execution_outputs_with_json_escaped_controls() {
        let mut payload = execution_json();
        payload["stdout"] = json!("\0".repeat(MAX_OUTPUT_BYTES));
        payload["stderr"] = json!("\0".repeat(MAX_OUTPUT_BYTES));
        let app = Router::new().route(
            "/execution",
            get(move || {
                let payload = payload.clone();
                async move { Json(payload) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let response = reqwest::get(format!("http://{address}/execution"))
            .await
            .expect("read mock response");

        let execution = decode_response(response)
            .await
            .expect("maximum valid execution outputs should fit the response cap");
        assert_eq!(execution.stdout.len(), MAX_OUTPUT_BYTES);
        assert_eq!(execution.stderr.len(), MAX_OUTPUT_BYTES);
        task.abort();
    }

    #[tokio::test]
    async fn retrieves_tenant_quota_from_authenticated_api_route() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/quota", get(capture_quota))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let quota = client.get_quota().await.expect("tenant quota");
        task.abort();

        let (headers, _) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        assert_eq!(quota.used_units, 7);
        assert_eq!(quota.active_limit, 8);
    }

    #[tokio::test]
    async fn wait_execution_returns_when_execution_becomes_terminal() {
        let calls = SequenceCounter::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/executions/{id}", get(sequence_execution))
            .with_state(calls.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let server = MpakaMcpServer::new(ApiClient::new(config).expect("API client"));

        let result = server
            .poll_execution_until_terminal(
                Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                Duration::from_secs(2),
                Duration::from_millis(100),
            )
            .await
            .expect("execution reaches terminal state");
        task.abort();

        assert_eq!(result.execution.status, "completed");
        assert!(!result.timed_out);
        assert_eq!(calls.0.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn wait_execution_returns_latest_state_when_deadline_expires() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new().route("/v1/executions/{id}", get(pending_execution));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let server = MpakaMcpServer::new(ApiClient::new(config).expect("API client"));

        let result = server
            .poll_execution_until_terminal(
                Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                Duration::from_millis(100),
                Duration::from_millis(100),
            )
            .await
            .expect("wait returns current execution");
        task.abort();

        assert_eq!(result.execution.status, "running");
        assert!(result.timed_out);
    }

    #[tokio::test]
    async fn wait_environment_returns_when_environment_is_terminal() {
        let calls = SequenceCounter::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/environments/{id}", get(sequence_environment))
            .with_state(calls.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let server = MpakaMcpServer::new(ApiClient::new(config).expect("API client"));

        let result = server
            .poll_environment_until_terminal(
                Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                Duration::from_secs(2),
                Duration::from_millis(100),
            )
            .await
            .expect("environment reaches terminal state");
        task.abort();

        assert_eq!(result.environment.status, "ready");
        assert!(!result.timed_out);
        assert_eq!(calls.0.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn wait_environment_returns_latest_state_when_deadline_expires() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new().route("/v1/environments/{id}", get(pending_environment));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let server = MpakaMcpServer::new(ApiClient::new(config).expect("API client"));

        let result = server
            .poll_environment_until_terminal(
                Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                Duration::from_millis(100),
                Duration::from_millis(100),
            )
            .await
            .expect("wait returns current environment");
        task.abort();

        assert_eq!(result.environment.status, "building");
        assert!(result.timed_out);
    }

    #[tokio::test]
    async fn forwards_execution_history_filters_to_api() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/executions", get(capture_execution_list))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let page = client
            .list_executions(
                Some(2),
                Some(10),
                Some("completed"),
                Some("python"),
                None,
                None,
            )
            .await
            .expect("execution page");
        task.abort();

        let (headers, request) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        let query = request["query"].as_str().unwrap();
        assert!(query.contains("page=2"));
        assert!(query.contains("page_size=10"));
        assert!(query.contains("status=completed"));
        assert!(query.contains("language=python"));
        assert_eq!(page.page, 2);
        assert_eq!(page.items[0].status, "completed");
        assert!(page.has_next);
    }

    #[tokio::test]
    async fn forwards_environment_page_to_tenant_api() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/environments", get(capture_environment_list))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let page = client
            .list_environments(Some(2), Some(10))
            .await
            .expect("environment page");
        task.abort();

        let (headers, request) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        let query = request["query"].as_str().unwrap();
        assert!(query.contains("page=2"));
        assert!(query.contains("page_size=10"));
        assert_eq!(page.page, 2);
        assert_eq!(page.items[0].status, "queued");
        assert!(page.has_next);
    }

    #[tokio::test]
    async fn rejects_oversized_error_response_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new().route(
            "/v1/executions/{id}",
            get(|| async {
                (
                    StatusCode::BAD_GATEWAY,
                    [("content-type", "application/json")],
                    "x".repeat(MAX_ERROR_RESPONSE_BYTES + 1),
                )
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");
        let id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

        let error = client.get_execution(id).await.expect_err("body over limit");
        task.abort();

        assert_eq!(error.code, "upstream_response_too_large");
    }

    #[tokio::test]
    async fn forwards_usage_period_and_page_size() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/usage", get(capture_usage))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");
        let since = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let until = DateTime::parse_from_rfc3339("2026-11-05T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        client
            .get_usage(Some(since), Some(until), Some(25), Some(3), None)
            .await
            .expect("usage response");
        task.abort();

        let (_, body) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        let query = body["query"].as_str().expect("query string");
        assert!(query.contains("since=2026-11-01T00%3A00%3A00%2B00%3A00"));
        assert!(query.contains("until=2026-11-05T00%3A00%3A00%2B00%3A00"));
        assert!(query.contains("limit=25"));
        assert!(query.contains("offset=3"));
    }

    #[tokio::test]
    async fn forwards_usage_cursor_without_offset() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/usage", get(capture_usage))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        client
            .get_usage(None, None, Some(25), None, Some("opaque-token".into()))
            .await
            .expect("usage response");
        task.abort();

        let (_, body) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        let query = body["query"].as_str().expect("query string");
        assert!(query.contains("cursor=opaque-token"));
        assert!(!query.contains("offset="));
    }

    #[tokio::test]
    async fn rejects_usage_cursor_with_nonzero_offset_before_request() {
        let config = McpConfig::new(
            Url::parse("http://127.0.0.1:1").expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let error = client
            .get_usage(None, None, None, Some(1), Some("opaque-token".into()))
            .await
            .expect_err("ambiguous usage pagination");

        assert_eq!(error.code, "invalid_input");
    }

    #[tokio::test]
    async fn rejects_oversized_usage_cursor_before_request() {
        let config = McpConfig::new(
            Url::parse("http://127.0.0.1:1").expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let error = client
            .get_usage(None, None, None, None, Some("x".repeat(513)))
            .await
            .expect_err("oversized usage cursor");

        assert_eq!(error.code, "invalid_input");
    }

    #[tokio::test]
    async fn rejects_invalid_usage_period_before_request() {
        let config = McpConfig::new(
            Url::parse("http://127.0.0.1:1").expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");
        let since = DateTime::parse_from_rfc3339("2026-11-05T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let until = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let error = client
            .get_usage(Some(since), Some(until), Some(25), Some(0), None)
            .await
            .expect_err("reversed period");

        assert_eq!(error.code, "invalid_input");
    }

    #[test]
    fn usage_query_enforces_api_period_and_limit_bounds() {
        let now = DateTime::parse_from_rfc3339("2026-11-25T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let too_old = now - chrono::Duration::days(367);
        let future = now + chrono::Duration::seconds(1);
        let invalid_queries = [
            (now, now, 10),
            (too_old, now, 10),
            (now - chrono::Duration::days(1), future, 10),
            (now - chrono::Duration::days(1), now, 0),
            (now - chrono::Duration::days(1), now, 1001),
        ];

        for (since, until, limit) in invalid_queries {
            assert_eq!(
                validate_usage_query(since, until, limit, now)
                    .expect_err("invalid usage query")
                    .code,
                "invalid_input"
            );
        }
        assert!(validate_usage_query(now - chrono::Duration::days(366), now, 1000, now).is_ok());
    }

    #[test]
    fn usage_input_rejects_malformed_timestamps() {
        assert_eq!(
            parse_usage_timestamp(Some("not a timestamp".into()), "since")
                .expect_err("malformed timestamp")
                .code,
            "invalid_input"
        );
    }

    #[test]
    fn wait_bounds_reject_unbounded_or_invalid_polling() {
        assert!(validate_wait_bounds(None, None).is_ok());
        assert!(validate_wait_bounds(Some(30_000), Some(5_000)).is_ok());
        assert!(validate_wait_bounds(Some(30_001), Some(500)).is_err());
        assert!(validate_wait_bounds(Some(1_000), Some(5_001)).is_err());
        assert!(validate_wait_bounds(Some(100), Some(101)).is_err());
    }

    #[test]
    fn tool_catalog_advertises_output_schemas_and_usage_filters() {
        let tools = tool_catalog();
        for name in [
            "execute_python",
            "get_execution",
            "cancel_execution",
            "create_environment",
            "get_environment",
            "get_usage",
            "get_usage_summary",
            "get_resource_profiles",
            "get_quota",
            "list_environments",
            "list_executions",
            "wait_execution",
            "wait_environment",
        ] {
            let tool = tools.iter().find(|tool| tool.name == name).expect("tool");
            assert!(
                tool.output_schema.is_some(),
                "missing output schema: {name}"
            );
        }
        let execution = tools
            .iter()
            .find(|tool| tool.name == "get_execution")
            .expect("execution tool");
        assert!(
            execution
                .output_schema
                .as_ref()
                .unwrap()
                .get("properties")
                .unwrap()
                .get("output_truncated")
                .is_some()
        );
        let usage = tools
            .iter()
            .find(|tool| tool.name == "get_usage")
            .expect("usage tool");
        let properties = usage.input_schema.get("properties").unwrap();
        assert!(properties.get("since").is_some());
        assert!(properties.get("until").is_some());
        assert!(properties.get("limit").is_some());
        assert!(properties.get("offset").is_some());
        let usage_schema = usage.output_schema.as_ref().unwrap();
        let schema_json = serde_json::Value::Object(usage_schema.as_ref().clone());
        assert!(
            schema_json
                .to_string()
                .contains("metering_contract_version")
        );
        assert!(schema_json["properties"].get("host_id").is_none());
        assert!(
            usage
                .description
                .as_deref()
                .unwrap()
                .contains("reuse the returned until")
        );
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.name == "get_usage")
                .unwrap()
                .annotations
                .as_ref()
                .unwrap()
                .read_only_hint,
            Some(true)
        );
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.name == "execute_python")
                .unwrap()
                .annotations
                .as_ref()
                .unwrap()
                .read_only_hint,
            Some(false)
        );
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.name == "cancel_execution")
                .unwrap()
                .annotations
                .as_ref()
                .unwrap()
                .destructive_hint,
            Some(true)
        );
        for name in [
            "get_quota",
            "list_environments",
            "list_executions",
            "wait_execution",
            "wait_environment",
        ] {
            let tool = tools.iter().find(|tool| tool.name == name).expect("tool");
            assert_eq!(
                tool.annotations.as_ref().unwrap().read_only_hint,
                Some(true),
                "{name} should be annotated as read-only"
            );
            assert!(tool.output_schema.is_some());
        }
    }

    #[test]
    fn tool_semaphore_caps_in_flight_operations_at_eight() {
        let config = McpConfig::new(
            Url::parse("http://127.0.0.1:5300").unwrap(),
            "test-key".into(),
            Duration::from_secs(5),
        )
        .unwrap();
        let server = MpakaMcpServer::new(ApiClient::new(config).unwrap());
        assert_eq!(
            server.tool_permits.available_permits(),
            MAX_CONCURRENT_TOOL_OPERATIONS
        );
    }

    #[tokio::test]
    async fn submits_python_with_bearer_key_and_stable_idempotency_key() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/executions", post(capture_submission))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let code = "x".repeat(256 * 1024 + 1);
        let execution = client
            .submit_execution(
                code.clone(),
                "operation-42".into(),
                Some(15_000),
                Some("custom_memory".into()),
                Some("custom_cpu".into()),
                Some(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()),
            )
            .await
            .expect("execution accepted");
        task.abort();

        let (headers, body) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        assert_eq!(headers["idempotency-key"], "operation-42");
        assert_eq!(body["language"], "python");
        assert_eq!(body["code"].as_str().unwrap().len(), code.len());
        assert_eq!(body["memory_profile"], "custom_memory");
        assert_eq!(body["cpu_profile"], "custom_cpu");
        assert_eq!(body["timeout_ms"], 15_000);
        assert_eq!(
            body["environment_id"],
            "00000000-0000-0000-0000-000000000002"
        );
        assert_eq!(
            execution.id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
        );
    }

    #[tokio::test]
    async fn creates_environment_with_pinned_requirements_and_idempotency_key() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/environments", post(capture_environment_creation))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let environment = client
            .create_environment(vec!["Requests==2.32.3".into()], "env-operation-42".into())
            .await
            .expect("environment accepted");
        task.abort();

        let (headers, body) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        assert_eq!(headers["idempotency-key"], "env-operation-42");
        assert_eq!(body["requirements"][0], "requests==2.32.3");
        assert_eq!(environment.status, "queued");
    }

    #[tokio::test]
    async fn requests_durable_execution_cancellation() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let execution_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let route = format!("/v1/executions/{execution_id}/cancel");
        let app = Router::new()
            .route(&route, post(capture_cancellation))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let execution = client
            .cancel_execution(execution_id)
            .await
            .expect("cancellation requested");
        task.abort();

        let (headers, _) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        assert_eq!(execution.status, "queued");
    }

    #[tokio::test]
    async fn retrieves_environment_and_usage_over_tenant_api_routes() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let environment_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let app = Router::new()
            .route("/v1/environments/{id}", get(get_environment_response))
            .route("/v1/usage", get(capture_usage))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");

        let environment = client
            .get_environment(environment_id)
            .await
            .expect("environment");
        let usage = client
            .get_usage(None, None, None, None, None)
            .await
            .expect("usage");
        task.abort();

        let (_, captured_query) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured request");
        let query = captured_query["query"].as_str().expect("query string");
        let parameters: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(query.as_bytes())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
        assert_eq!(parameters.get("limit").map(String::as_str), Some("100"));
        assert_eq!(parameters.get("offset").map(String::as_str), Some("0"));
        let since = DateTime::parse_from_rfc3339(&parameters["since"]).unwrap();
        let until = DateTime::parse_from_rfc3339(&parameters["until"]).unwrap();
        assert_eq!(until - since, chrono::Duration::days(30));

        assert_eq!(environment.id, environment_id);
        assert_eq!(usage.attempt_count, 1);
        assert_eq!(usage.provisioned_vcpu_ms, 2000);
        assert_eq!(usage.items[0].metering_contract_version, 1);
    }

    #[tokio::test]
    async fn retrieves_aggregate_usage_summary_with_daily_rows() {
        let captured = CapturedRequest::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock API");
        let address = listener.local_addr().expect("mock API address");
        let app = Router::new()
            .route("/v1/usage/summary", get(capture_usage_summary))
            .with_state(captured.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock API");
        });
        let config = McpConfig::new(
            Url::parse(&format!("http://{address}")).expect("mock API URL"),
            "tenant-key-for-test".into(),
            Duration::from_secs(5),
        )
        .expect("valid MCP config");
        let client = ApiClient::new(config).expect("API client");
        let since = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let until = DateTime::parse_from_rfc3339("2026-11-24T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let summary = client
            .get_usage_summary(Some(since), Some(until), Some("day".into()))
            .await
            .expect("usage summary");
        task.abort();

        let (headers, request) = captured
            .0
            .lock()
            .expect("capture lock")
            .take()
            .expect("captured summary request");
        assert_eq!(headers["authorization"], "Bearer tenant-key-for-test");
        assert!(request["query"].as_str().unwrap().contains("group_by=day"));
        assert_eq!(summary.attempt_count, 3);
        assert_eq!(summary.pending_attempt_count, 1);
        assert_eq!(summary.daily.len(), 1);
        assert_eq!(summary.daily[0].date, "2026-11-23");
    }

    #[test]
    fn marks_large_execution_output_as_truncated() {
        let mut execution: Execution = serde_json::from_value(execution_json()).expect("execution");
        execution.stdout = format!("{}終", "x".repeat(MAX_OUTPUT_BYTES - 1));

        let call_result = tool_success(&execution);
        let text = match &call_result.content[0] {
            ContentBlock::Text(text) => text.text.as_str(),
            _ => panic!("expected text result"),
        };
        assert_eq!(
            call_result.structured_content.as_ref().unwrap()["output_truncated"],
            true
        );
        let result: serde_json::Value = serde_json::from_str(text).expect("JSON tool result");
        assert_eq!(result["output_truncated"], true);
        assert_eq!(&result, call_result.structured_content.as_ref().unwrap());
        assert!(
            result["stdout"]
                .as_str()
                .unwrap()
                .ends_with("[output truncated by mPaka MCP]")
        );
    }
}
