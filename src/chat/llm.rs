//! LLM client for OpenAI-compatible APIs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::chat::config::EndpointConfig;

/// LLM client for chat completions
pub struct LlmClient {
    client: reqwest::Client,
    config: EndpointConfig,
}

impl LlmClient {
    /// Create a new LLM client
    pub fn new(config: EndpointConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            // Avoid system proxy discovery to prevent macOS SystemConfiguration panics.
            .no_proxy()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()?;

        Ok(Self { client, config })
    }

    /// Test connectivity to the LLM endpoint
    pub async fn test_connection(&self) -> Result<()> {
        let url = format!("{}/models", self.config.base_url);

        let mut req = self.client.get(&url);
        if let Some(api_key) = &self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req
            .send()
            .await
            .context("Failed to connect to LLM endpoint")?;

        if !response.status().is_success() {
            anyhow::bail!("LLM endpoint returned error: {}", response.status());
        }

        Ok(())
    }

    /// Send a chat completion request
    pub async fn chat_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<ChatCompletionResponse> {
        let url = format!("{}/chat/completions", self.config.base_url);

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            tools,
            tool_choice: None,
            temperature: Some(0.7),
            max_tokens: None,
        };

        let mut req = self.client.post(&url).json(&request);
        if let Some(api_key) = &self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req
            .send()
            .await
            .context("Failed to send chat completion request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Chat completion failed with status {}: {}", status, text);
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .context("Failed to parse chat completion response")?;

        Ok(completion)
    }
}

/// Chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }
}

/// Tool call from assistant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Tool definition for the API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Chat completion request
#[derive(Debug, Clone, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

/// Chat completion response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<ChatChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub message: ChatMessage,
}

/// Build system prompt for Interest chat
pub fn build_system_prompt() -> String {
    r#"You are an AI assistant for Interest, a Brazilian B3 stock exchange investment tracker.

You have access to tools that can:
- Show portfolio positions and performance
- Calculate taxes (swing trade, IRPF)
- Import transaction data
- Manage corporate actions
- Query income events
- And more

How interest works
- The non-sql tools do fairly complex processing of the data
- For instance, when looking at how much quantity there is today it needs to understand which corporate actions there were along the way and adjust for them
- That means data such as quantities, cost basis, and more, are not trivial to obtain from the database directly

When the user asks about their investments:
1. Use the appropriate Interest tools to fetch data
2. Analyze and explain the results in a clear, helpful way
3. For financial calculations, always verify the numbers

Tool usage guidelines:
- IMPORTANT: Prefer Interest high level tools over raw SQL queries
- For database exploration, use db_schema first
- For custom queries, use db_query (read-only)
- Always explain your reasoning before showing results
- When answering questions do not simply show big lists of transactions, use your tools to learn what is being asked and respond
- Tool output_mode:
  - Default to "capture" unless the user explicitly asks to see raw output
  - Use "present" when the user wants the full table/JSON shown
  - Use "capture_and_present" only when you need both
- Output rendering is in a terminal with termimad (most Markdown supported), but Markdown math is not supported
- Intent guidance:
  - "how much of <asset>" should include both quantity and current value
  - "how many <asset>" should include quantity only

Remember:
- All currency values are in BRL (Brazilian Reais)
- Tax rules follow Brazilian regulations (15% swing trade with R$20k/month exemption for stocks, 20% for FIIs, etc.)
- Performance calculations use time-weighted return (TWR)
- SQL queries should be your last resort, and only to answer simpler questions that do not require the processing done for portfolio, performance, tax calculations"#.to_string()
}
