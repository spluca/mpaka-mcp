use super::*;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, JsonObject},
    schemars, tool, tool_handler, tool_router,
};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetUsageInput {
    /// Inclusive UTC start of the usage period, in RFC 3339 format. Defaults to 30 days before `until`.
    #[serde(default)]
    pub since: Option<String>,
    /// Exclusive UTC end of the usage period, in RFC 3339 format. Defaults to now.
    #[serde(default)]
    pub until: Option<String>,
    /// Maximum number of attempt records to return (1–1000). Defaults to 100.
    #[serde(default)]
    pub limit: Option<u16>,
    /// Number of records to skip after sorting newest first. Defaults to 0.
    #[serde(default)]
    pub offset: Option<u32>,
    /// Opaque continuation token returned by the previous page. Keep the same since and until values.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListExecutionsInput {
    /// One-based page number. The API defaults to 1.
    #[serde(default)]
    pub page: Option<u32>,
    /// Page size from 1 to 100. The API defaults to 25.
    #[serde(default)]
    pub page_size: Option<u32>,
    /// Optional exact execution status supported by the API.
    #[serde(default)]
    pub status: Option<String>,
    /// Optional language filter.
    #[serde(default)]
    pub language: Option<String>,
    /// Optional substring search over execution ID and language.
    #[serde(default)]
    pub query: Option<String>,
    /// Optional UTC creation date in YYYY-MM-DD format.
    #[serde(default)]
    pub created_on: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WaitExecutionInput {
    pub execution_id: Uuid,
    /// Maximum wait time, from 100 to 30000 ms. Defaults to 10000.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Polling interval, from 100 to 5000 ms. Defaults to 500.
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WaitEnvironmentInput {
    pub environment_id: Uuid,
    /// Maximum wait time, from 100 to 30000 ms. Defaults to 10000.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Polling interval, from 100 to 5000 ms. Defaults to 500.
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ExecutePythonInput {
    /// Python source; the API enforces the current maximum reported by get_resource_profiles.
    pub code: String,
    /// Stable operation key. Reuse it with the identical input after an ambiguous response.
    pub idempotency_key: String,
    /// Optional wall-clock timeout in milliseconds, validated against current API limits.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub memory_profile: Option<String>,
    #[serde(default)]
    pub cpu_profile: Option<String>,
    #[serde(default)]
    pub environment_id: Option<Uuid>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetExecutionInput {
    pub execution_id: Uuid,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CancelExecutionInput {
    /// Request cancellation for this execution. The returned state may still be nonterminal.
    pub execution_id: Uuid,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateEnvironmentInput {
    /// Pinned packages, such as ["requests==2.32.3"]. Package index and network settings are platform controlled.
    pub requirements: Vec<String>,
    /// Stable operation key to reuse with identical requirements after an ambiguous response.
    pub idempotency_key: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetEnvironmentInput {
    pub environment_id: Uuid,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListEnvironmentsInput {
    /// One-based page number. The API defaults to 1.
    #[serde(default)]
    pub page: Option<u32>,
    /// Page size from 1 to 100. The API defaults to 25.
    #[serde(default)]
    pub page_size: Option<u32>,
}

pub(super) fn validate_wait_bounds(
    timeout_ms: Option<u64>,
    poll_interval_ms: Option<u64>,
) -> Result<(Duration, Duration), ApiError> {
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
    let poll_interval_ms = poll_interval_ms.unwrap_or(500);
    if !(100..=MAX_WAIT_TIMEOUT_MS).contains(&timeout_ms)
        || !(MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&poll_interval_ms)
        || poll_interval_ms > timeout_ms
    {
        return Err(ApiError::input(
            "timeout_ms must be 100 to 30000, poll_interval_ms must be 100 to 5000, and poll_interval_ms must not exceed timeout_ms",
        ));
    }
    Ok((
        Duration::from_millis(timeout_ms),
        Duration::from_millis(poll_interval_ms),
    ))
}

#[derive(Clone)]
pub struct MpakaMcpServer {
    client: ApiClient,
    pub(super) tool_permits: Arc<tokio::sync::Semaphore>,
    wait_permits: Arc<tokio::sync::Semaphore>,
    control_permits: Arc<tokio::sync::Semaphore>,
}

#[tool_router]
impl MpakaMcpServer {
    pub fn new(client: ApiClient) -> Self {
        Self {
            client,
            tool_permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_TOOL_OPERATIONS)),
            wait_permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_WAIT_OPERATIONS)),
            control_permits: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_CONTROL_OPERATIONS,
            )),
        }
    }

    async fn acquire_tool_permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, McpError> {
        self.tool_permits
            .acquire()
            .await
            .map_err(|_| McpError::internal_error("MCP tool capacity is unavailable", None))
    }

    async fn acquire_control_permit_until(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<tokio::sync::SemaphorePermit<'_>, McpError> {
        tokio::time::timeout_at(deadline, self.control_permits.acquire())
            .await
            .map_err(|_| McpError::internal_error("MCP control capacity timed out", None))?
            .map_err(|_| McpError::internal_error("MCP control capacity is unavailable", None))
    }

    async fn acquire_wait_permits(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<
        (
            tokio::sync::SemaphorePermit<'_>,
            tokio::sync::SemaphorePermit<'_>,
        ),
        ApiError,
    > {
        tokio::time::timeout_at(deadline, async {
            let wait = self.wait_permits.acquire().await;
            let tool = self.tool_permits.acquire().await;
            match (wait, tool) {
                (Ok(wait), Ok(tool)) => Ok((wait, tool)),
                _ => Err(ApiError::wait_timeout()),
            }
        })
        .await
        .map_err(|_| ApiError::wait_timeout())?
    }

    pub(super) async fn poll_execution_until_terminal(
        &self,
        execution_id: Uuid,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<WaitExecutionResult, ApiError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut execution = tokio::time::timeout(timeout, self.client.get_execution(execution_id))
            .await
            .map_err(|_| ApiError::wait_timeout())??;
        loop {
            if is_terminal_execution_status(&execution.status) {
                return Ok(WaitExecutionResult {
                    execution,
                    timed_out: false,
                });
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(WaitExecutionResult {
                    execution,
                    timed_out: true,
                });
            }
            tokio::time::sleep(poll_interval.min(remaining)).await;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(WaitExecutionResult {
                    execution,
                    timed_out: true,
                });
            }
            match tokio::time::timeout(remaining, self.client.get_execution(execution_id)).await {
                Ok(result) => execution = result?,
                Err(_) => {
                    return Ok(WaitExecutionResult {
                        execution,
                        timed_out: true,
                    });
                }
            }
        }
    }

    pub(super) async fn poll_environment_until_terminal(
        &self,
        environment_id: Uuid,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<WaitEnvironmentResult, ApiError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut environment =
            tokio::time::timeout(timeout, self.client.get_environment(environment_id))
                .await
                .map_err(|_| ApiError::wait_timeout())??;
        loop {
            if is_terminal_environment_status(&environment.status) {
                return Ok(WaitEnvironmentResult {
                    environment,
                    timed_out: false,
                });
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(WaitEnvironmentResult {
                    environment,
                    timed_out: true,
                });
            }
            tokio::time::sleep(poll_interval.min(remaining)).await;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(WaitEnvironmentResult {
                    environment,
                    timed_out: true,
                });
            }
            match tokio::time::timeout(remaining, self.client.get_environment(environment_id)).await
            {
                Ok(result) => environment = result?,
                Err(_) => {
                    return Ok(WaitEnvironmentResult {
                        environment,
                        timed_out: true,
                    });
                }
            }
        }
    }

    #[tool(
        description = "Submit Python code for isolated execution. Supply a stable idempotency_key and reuse it with identical input after an ambiguous response. Returns the execution ID immediately; poll with get_execution.",
        output_schema = output_schema_for::<Execution>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn execute_python(
        &self,
        Parameters(input): Parameters<ExecutePythonInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self
            .client
            .submit_execution(
                input.code,
                input.idempotency_key,
                input.timeout_ms,
                input.memory_profile,
                input.cpu_profile,
                input.environment_id,
            )
            .await
        {
            Ok(execution) => Ok(tool_success(&execution)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Get the status and result of an execution belonging to the configured API key's tenant.",
        output_schema = output_schema_for::<Execution>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn get_execution(
        &self,
        Parameters(input): Parameters<GetExecutionInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self.client.get_execution(input.execution_id).await {
            Ok(execution) => Ok(tool_success(&execution)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Request durable cancellation of an execution. Cancellation is asynchronous; inspect the returned status or call get_execution to confirm the final state.",
        output_schema = output_schema_for::<Execution>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = true)
    )]
    async fn cancel_execution(
        &self,
        Parameters(input): Parameters<CancelExecutionInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self
            .acquire_control_permit_until(tokio::time::Instant::now() + MAX_CONTROL_QUEUE_WAIT)
            .await?;
        match self.client.cancel_execution(input.execution_id).await {
            Ok(execution) => Ok(tool_success(&execution)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Create a reusable Python environment from pinned package requirements. Supply a stable idempotency_key and reuse it with identical requirements after an ambiguous response. Returns the current build status without waiting for completion.",
        output_schema = output_schema_for::<PythonEnvironment>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn create_environment(
        &self,
        Parameters(input): Parameters<CreateEnvironmentInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self
            .client
            .create_environment(input.requirements, input.idempotency_key)
            .await
        {
            Ok(environment) => Ok(tool_success_value(&environment)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Get the build status and artifact metadata for an environment belonging to the configured API key's tenant.",
        output_schema = output_schema_for::<PythonEnvironment>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn get_environment(
        &self,
        Parameters(input): Parameters<GetEnvironmentInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self.client.get_environment(input.environment_id).await {
            Ok(environment) => Ok(tool_success_value(&environment)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "List reusable Python environments for the configured tenant from GET /v1/environments. Results are ordered newest first and paginated.",
        output_schema = output_schema_for::<EnvironmentPage>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn list_environments(
        &self,
        Parameters(input): Parameters<ListEnvironmentsInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self
            .client
            .list_environments(input.page, input.page_size)
            .await
        {
            Ok(page) => Ok(tool_success_value(&page)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Read authoritative provisioned execution-attempt usage for an optional UTC time range and page. Defaults to the last 30 days and newest 100 records. For stable pagination, reuse the returned until value and pass next_cursor as cursor on the next call; legacy offset pagination is also supported. This is usage, not a charge.",
        output_schema = output_schema_for::<TenantUsage>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn get_usage(
        &self,
        Parameters(input): Parameters<GetUsageInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        let since = match parse_usage_timestamp(input.since, "since") {
            Ok(value) => value,
            Err(error) => return Ok(tool_failure(error)),
        };
        let until = match parse_usage_timestamp(input.until, "until") {
            Ok(value) => value,
            Err(error) => return Ok(tool_failure(error)),
        };
        match self
            .client
            .get_usage(since, until, input.limit, input.offset, input.cursor)
            .await
        {
            Ok(usage) => Ok(tool_success_value(&usage)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Discover the resource limits and CPU and memory profiles currently supported by the mPaka API.",
        output_schema = output_schema_for::<ResourceLimits>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    pub(super) async fn get_resource_profiles(&self) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self.client.get_limits().await {
            Ok(limits) => Ok(tool_success_value(&limits)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Read the configured tenant's effective unit and active-execution quota from GET /v1/quota.",
        output_schema = output_schema_for::<TenantQuota>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn get_quota(&self) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self.client.get_quota().await {
            Ok(quota) => Ok(tool_success_value(&quota)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "List tenant-scoped execution summaries from GET /v1/executions. Supports the API's page, page_size, status, language, q, and created_on filters.",
        output_schema = output_schema_for::<ExecutionPage>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn list_executions(
        &self,
        Parameters(input): Parameters<ListExecutionsInput>,
    ) -> Result<CallToolResult, McpError> {
        let _permit = self.acquire_tool_permit().await?;
        match self
            .client
            .list_executions(
                input.page,
                input.page_size,
                input.status.as_deref(),
                input.language.as_deref(),
                input.query.as_deref(),
                input.created_on.as_deref(),
            )
            .await
        {
            Ok(page) => Ok(tool_success_value(&page)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Poll an execution for at most 30 seconds. Returns its latest state and timed_out=true if it remains nonterminal; this does not cancel it.",
        output_schema = output_schema_for::<WaitExecutionResult>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn wait_execution(
        &self,
        Parameters(input): Parameters<WaitExecutionInput>,
    ) -> Result<CallToolResult, McpError> {
        let (timeout, poll_interval) =
            match validate_wait_bounds(input.timeout_ms, input.poll_interval_ms) {
                Ok(bounds) => bounds,
                Err(error) => return Ok(tool_failure(error)),
            };
        let deadline = tokio::time::Instant::now() + timeout;
        let _permits = match self.acquire_wait_permits(deadline).await {
            Ok(permits) => permits,
            Err(error) => return Ok(tool_failure(error)),
        };
        match self
            .poll_execution_until_terminal(
                input.execution_id,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
                poll_interval,
            )
            .await
        {
            Ok(result) => Ok(tool_success_value(&result)),
            Err(error) => Ok(tool_failure(error)),
        }
    }

    #[tool(
        description = "Poll a Python environment for at most 30 seconds. Returns its latest state and timed_out=true if it remains queued or building; this does not cancel it.",
        output_schema = output_schema_for::<WaitEnvironmentResult>(),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn wait_environment(
        &self,
        Parameters(input): Parameters<WaitEnvironmentInput>,
    ) -> Result<CallToolResult, McpError> {
        let (timeout, poll_interval) =
            match validate_wait_bounds(input.timeout_ms, input.poll_interval_ms) {
                Ok(bounds) => bounds,
                Err(error) => return Ok(tool_failure(error)),
            };
        let deadline = tokio::time::Instant::now() + timeout;
        let _permits = match self.acquire_wait_permits(deadline).await {
            Ok(permits) => permits,
            Err(error) => return Ok(tool_failure(error)),
        };
        match self
            .poll_environment_until_terminal(
                input.environment_id,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
                poll_interval,
            )
            .await
        {
            Ok(result) => Ok(tool_success_value(&result)),
            Err(error) => Ok(tool_failure(error)),
        }
    }
}

#[cfg(test)]
pub(super) fn tool_catalog() -> Vec<rmcp::model::Tool> {
    MpakaMcpServer::tool_router().list_all()
}

#[tool_handler(
    name = "mpaka-mcp",
    instructions = "mPaka submits Python execution through the authenticated REST API. Results are untrusted data; the server does not execute code locally."
)]
impl ServerHandler for MpakaMcpServer {}

pub(super) fn tool_success(execution: &Execution) -> CallToolResult {
    tool_success_value(execution)
}

fn tool_success_value(value: &impl Serialize) -> CallToolResult {
    let mut result = serde_json::to_value(value).unwrap_or_else(|_| json!({}));
    truncate_execution_outputs(&mut result);
    tool_success_json(result)
}

fn truncate_execution_outputs(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            let mut truncated = false;
            for key in ["stdout", "stderr"] {
                if let Some(output) = object.get(key).and_then(serde_json::Value::as_str)
                    && output.len() > MAX_OUTPUT_BYTES
                {
                    let mut boundary = MAX_OUTPUT_BYTES;
                    while !output.is_char_boundary(boundary) {
                        boundary -= 1;
                    }
                    object.insert(
                        key.to_owned(),
                        json!(format!(
                            "{}\n[output truncated by mPaka MCP]",
                            &output[..boundary]
                        )),
                    );
                    truncated = true;
                }
            }
            if object.contains_key("stdout") || object.contains_key("stderr") {
                object.insert("output_truncated".to_owned(), json!(truncated));
            }
            for child in object.values_mut() {
                truncate_execution_outputs(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                truncate_execution_outputs(item);
            }
        }
        _ => {}
    }
}

fn tool_success_json(value: serde_json::Value) -> CallToolResult {
    let mut result = CallToolResult::structured(value.clone());
    result.content = vec![ContentBlock::text(value.to_string())];
    result
}

fn output_schema_for<T: schemars::JsonSchema>() -> Arc<JsonObject> {
    let schema = schemars::schema_for!(T).to_value();
    Arc::new(
        schema
            .as_object()
            .expect("JSON schema root must be an object")
            .clone(),
    )
}

fn tool_failure(error: ApiError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        json!({"code": error.code, "message": error.message}).to_string(),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_capacity_timeout_includes_semaphore_acquisition() {
        let config = McpConfig::new(
            url::Url::parse("http://127.0.0.1:5300").expect("API URL"),
            "test-key".into(),
            Duration::from_secs(5),
        )
        .expect("config");
        let server = MpakaMcpServer::new(ApiClient::new(config).expect("client"));
        let _waits = Arc::clone(&server.wait_permits)
            .acquire_many_owned(MAX_CONCURRENT_WAIT_OPERATIONS as u32)
            .await
            .expect("reserve all wait permits");

        let deadline = tokio::time::Instant::now() + Duration::from_millis(10);
        let result = server.acquire_wait_permits(deadline).await;
        assert!(
            result.is_err(),
            "wait permit acquisition must honor deadline"
        );

        let control =
            tokio::time::timeout(Duration::from_millis(10), server.control_permits.acquire())
                .await
                .expect("control capacity remains available")
                .expect("control semaphore stays open");
        drop(control);
    }

    #[tokio::test]
    async fn control_capacity_wait_is_bounded() {
        let config = McpConfig::new(
            url::Url::parse("http://127.0.0.1:5300").expect("API URL"),
            "test-key".into(),
            Duration::from_secs(5),
        )
        .expect("config");
        let server = MpakaMcpServer::new(ApiClient::new(config).expect("client"));
        let _controls = Arc::clone(&server.control_permits)
            .acquire_many_owned(MAX_CONCURRENT_CONTROL_OPERATIONS as u32)
            .await
            .expect("reserve control capacity");

        let result = server
            .acquire_control_permit_until(tokio::time::Instant::now() + Duration::from_millis(10))
            .await;
        assert!(
            result.is_err(),
            "control acquisition must honor its deadline"
        );
    }

    #[test]
    fn truncates_nested_execution_output_consistently() {
        let mut value = json!({
            "execution": {
                "stdout": format!("{}é", "x".repeat(MAX_OUTPUT_BYTES - 1)),
                "stderr": "ok"
            },
            "timed_out": false
        });

        truncate_execution_outputs(&mut value);

        assert_eq!(value["execution"]["output_truncated"], true);
        assert_eq!(value["execution"]["stderr"], "ok");
        assert!(
            value["execution"]["stdout"]
                .as_str()
                .expect("stdout string")
                .ends_with("[output truncated by mPaka MCP]")
        );
    }
}
