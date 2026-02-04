//! Chat-based TUI interface.

use anyhow::Result;
use colored::Colorize;
use rustyline::error::ReadlineError;
use std::io::Write;

use crate::chat::{
    approvals::{ApprovalDecision, ApprovalState, PolicyGroup},
    config::ChatConfig,
    llm::{build_system_prompt, ChatMessage, LlmClient},
    tools,
};
use crate::options::OutputOptions;
use crate::ui::readline;
use termimad::MadSkin;
use tracing::{debug, trace};

const MAX_TOOL_CALLS_PER_TURN: usize = 10;
const MAX_MESSAGE_HISTORY: usize = 50;

/// Launch the chat TUI
pub async fn launch_chat(output_options: OutputOptions) -> Result<()> {
    println!("{}", "Interest Chat - AI Assistant".bold().cyan());
    println!("Type your question or {} to exit\n", "/exit".cyan());

    // Load or create config
    let config_path = ChatConfig::config_path()?;
    let is_first_run = !config_path.exists();
    let mut config = ChatConfig::load()?;
    let mut approval_state = ApprovalState::new();

    // First-run setup if needed
    if is_first_run {
        println!("{}", "First-time setup:".yellow().bold());
        if !setup_endpoint(&mut config).await? {
            return Ok(());
        }
    }

    // Test connection
    println!("Testing connection to LLM endpoint...");
    let client = LlmClient::new(config.endpoint.clone())?;
    if let Err(e) = client.test_connection().await {
        eprintln!("{} Failed to connect: {}", "Error:".red().bold(), e);
        eprintln!(
            "\nRun {} to reconfigure the endpoint",
            "/chat config".cyan()
        );
        return Ok(());
    }
    println!("{} Connected successfully!\n", "✓".green().bold());

    // Initialize message history with system prompt
    let mut messages = vec![ChatMessage::system(build_system_prompt())];
    let mut turn_index: u64 = 0;

    // Get available tools
    let tool_definitions: Vec<_> = tools::get_all_tools()
        .iter()
        .map(|t| t.to_definition())
        .collect();

    let mut rl = readline::Readline::new(&[], None)?;
    let prompt = if output_options.is_color_enabled() {
        format!("\n{}", "chat> ".cyan().bold())
    } else {
        "\nchat> ".to_string()
    };

    loop {
        match rl.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Handle slash commands
                if trimmed.starts_with('/') {
                    if let Err(e) =
                        handle_slash_command(trimmed, &mut config, &mut approval_state, &client)
                            .await
                    {
                        eprintln!("{} {}", "Error:".red().bold(), e);
                    }
                    continue;
                }

                // Add user message
                turn_index += 1;
                debug!("chat turn {}: user input received", turn_index);
                trace!("chat turn {}: user message={}", turn_index, trimmed);
                messages.push(ChatMessage::user(trimmed));
                truncate_message_history(&mut messages);

                // Get completion from LLM
                trace!(
                    "chat turn {}: requesting completion messages={}",
                    turn_index,
                    serde_json::to_string(&messages)
                        .unwrap_or_else(|_| "<serialize_error>".to_string())
                );
                match client
                    .chat_completion(messages.clone(), Some(tool_definitions.clone()))
                    .await
                {
                    Ok(response) => {
                        debug!(
                            "chat turn {}: completion received choices={}",
                            turn_index,
                            response.choices.len()
                        );
                        trace!(
                            "chat turn {}: completion response={}",
                            turn_index,
                            serde_json::to_string(&response)
                                .unwrap_or_else(|_| "<serialize_error>".to_string())
                        );
                        if response.choices.is_empty() {
                            eprintln!("{} No response from LLM", "Error:".red().bold());
                            continue;
                        }

                        let choice = &response.choices[0];
                        let assistant_message = &choice.message;
                        trace!(
                            "chat turn {}: assistant message={}",
                            turn_index,
                            serde_json::to_string(&assistant_message)
                                .unwrap_or_else(|_| "<serialize_error>".to_string())
                        );

                        // Handle tool calls
                        if let Some(tool_calls) = &assistant_message.tool_calls {
                            debug!(
                                "chat turn {}: assistant requested {} tool call(s)",
                                turn_index,
                                tool_calls.len()
                            );
                            trace!(
                                "chat turn {}: tool calls={}",
                                turn_index,
                                serde_json::to_string(&tool_calls)
                                    .unwrap_or_else(|_| "<serialize_error>".to_string())
                            );
                            if tool_calls.len() > MAX_TOOL_CALLS_PER_TURN {
                                eprintln!(
                                    "{} Too many tool calls ({}). Limiting to {}.",
                                    "Warning:".yellow().bold(),
                                    tool_calls.len(),
                                    MAX_TOOL_CALLS_PER_TURN
                                );
                            }

                            // Add assistant message with tool calls
                            messages.push(assistant_message.clone());
                            truncate_message_history(&mut messages);

                            // Execute each tool call
                            for tool_call in tool_calls.iter().take(MAX_TOOL_CALLS_PER_TURN) {
                                let tool_name = &tool_call.function.name;
                                let arguments: serde_json::Value =
                                    serde_json::from_str(&tool_call.function.arguments)?;
                                let output_mode = tools::ToolOutputMode::from_arguments(&arguments);
                                let cleaned_arguments = tools::strip_output_mode(&arguments);
                                debug!(
                                    "chat turn {}: executing tool {} output_mode={:?}",
                                    turn_index, tool_name, output_mode
                                );
                                trace!(
                                    "chat turn {}: tool {} args={}",
                                    turn_index,
                                    tool_name,
                                    serde_json::to_string(&cleaned_arguments)
                                        .unwrap_or_else(|_| "<serialize_error>".to_string())
                                );

                                if output_mode.present() {
                                    println!(
                                        "\n{} Calling tool: {}",
                                        "🔧".cyan(),
                                        tool_name.yellow().bold()
                                    );
                                }

                                // Check if approval is needed
                                let tool = tools::get_tool(tool_name);
                                let policy_group = tool
                                    .as_ref()
                                    .map(|t| t.policy_group)
                                    .unwrap_or(PolicyGroup::Read);

                                let needs_approval = approval_state.should_prompt(
                                    tool_name,
                                    policy_group,
                                    &config.approvals,
                                );

                                if approval_state.is_denied(
                                    tool_name,
                                    policy_group,
                                    &config.approvals,
                                ) {
                                    println!(
                                        "{} Tool execution blocked by default deny policy",
                                        "✗".red().bold()
                                    );
                                    let tool_result =
                                        "Tool execution blocked by default deny policy".to_string();
                                    messages.push(ChatMessage::tool(&tool_call.id, tool_result));
                                    truncate_message_history(&mut messages);
                                    continue;
                                }

                                if needs_approval {
                                    match prompt_approval(tool_name, &cleaned_arguments)? {
                                        ApprovalDecision::AllowOnce => {
                                            // Continue with execution
                                        }
                                        ApprovalDecision::AllowSession => {
                                            approval_state.record_decision(
                                                tool_name,
                                                policy_group,
                                                ApprovalDecision::AllowSession,
                                            );
                                        }
                                        ApprovalDecision::AllowAlways => {
                                            config.add_always_allow(tool_name.to_string());
                                            config.save()?;
                                            println!(
                                                "{} Added {} to always-allow list",
                                                "✓".green().bold(),
                                                tool_name.yellow()
                                            );
                                        }
                                        ApprovalDecision::Cancel => {
                                            println!("{} Cancelled by user", "✗".red().bold());
                                            let tool_result =
                                                "Tool execution cancelled by user".to_string();
                                            messages.push(ChatMessage::tool(
                                                &tool_call.id,
                                                tool_result,
                                            ));
                                            continue;
                                        }
                                    }
                                }

                                // Execute tool
                                match tools::execute_tool(tool_name, cleaned_arguments, output_mode)
                                    .await
                                {
                                    Ok(result) => {
                                        debug!(
                                            "chat turn {}: tool {} completed presented={}",
                                            turn_index, tool_name, result.presented
                                        );
                                        trace!(
                                            "chat turn {}: tool {} result={}",
                                            turn_index,
                                            tool_name,
                                            result.content
                                        );
                                        if output_mode.present() && !result.presented {
                                            println!("\n{}", result.content);
                                        }
                                        messages
                                            .push(ChatMessage::tool(&tool_call.id, result.content));
                                        truncate_message_history(&mut messages);
                                    }
                                    Err(e) => {
                                        let error_msg = format!("Tool execution failed: {}", e);
                                        debug!(
                                            "chat turn {}: tool {} failed",
                                            turn_index, tool_name
                                        );
                                        trace!(
                                            "chat turn {}: tool {} error={}",
                                            turn_index,
                                            tool_name,
                                            error_msg
                                        );
                                        eprintln!("{} {}", "Error:".red().bold(), error_msg);
                                        messages.push(ChatMessage::tool(&tool_call.id, error_msg));
                                        truncate_message_history(&mut messages);
                                    }
                                }
                            }

                            // Get another completion after tool results
                            trace!(
                                "chat turn {}: requesting follow-up completion messages={}",
                                turn_index,
                                serde_json::to_string(&messages)
                                    .unwrap_or_else(|_| "<serialize_error>".to_string())
                            );
                            match client
                                .chat_completion(messages.clone(), Some(tool_definitions.clone()))
                                .await
                            {
                                Ok(response) => {
                                    debug!(
                                        "chat turn {}: follow-up completion received choices={}",
                                        turn_index,
                                        response.choices.len()
                                    );
                                    trace!(
                                        "chat turn {}: follow-up response={}",
                                        turn_index,
                                        serde_json::to_string(&response)
                                            .unwrap_or_else(|_| "<serialize_error>".to_string())
                                    );
                                    if let Some(choice) = response.choices.first() {
                                        if let Some(content) = &choice.message.content {
                                            trace!(
                                                "chat turn {}: follow-up assistant content={}",
                                                turn_index,
                                                content
                                            );
                                            render_assistant_message(content, &output_options);
                                            messages.push(choice.message.clone());
                                            truncate_message_history(&mut messages);
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("{} {}", "Error:".red().bold(), e);
                                }
                            }
                        } else if let Some(content) = &assistant_message.content {
                            // Regular text response
                            debug!("chat turn {}: assistant response", turn_index);
                            trace!("chat turn {}: assistant content={}", turn_index, content);
                            render_assistant_message(content, &output_options);
                            messages.push(assistant_message.clone());
                            truncate_message_history(&mut messages);
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {}", "Error:".red().bold(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                eprintln!("{} {}", "Error:".red().bold(), err);
                break;
            }
        }
    }

    Ok(())
}

fn render_assistant_message(content: &str, output_options: &OutputOptions) {
    println!();
    if output_options.is_color_enabled() {
        let skin = MadSkin::default();
        skin.print_text(content);
    } else {
        println!("{}", content);
    }
}

fn truncate_message_history(messages: &mut Vec<ChatMessage>) {
    if messages.len() <= MAX_MESSAGE_HISTORY {
        return;
    }

    let system = messages.first().cloned();
    let keep = MAX_MESSAGE_HISTORY.saturating_sub(1);
    let tail_start = messages.len().saturating_sub(keep);
    let mut trimmed = Vec::with_capacity(keep + 1);
    if let Some(system) = system {
        trimmed.push(system);
    }
    trimmed.extend(messages[tail_start..].iter().cloned());
    *messages = trimmed;
}

/// Setup endpoint configuration (first-run wizard)
async fn setup_endpoint(config: &mut ChatConfig) -> Result<bool> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        print!("Enter LLM endpoint URL [http://localhost:11434/v1]: ");
        stdout.flush()?;
        let mut endpoint = String::new();
        stdin.read_line(&mut endpoint)?;
        let endpoint = endpoint.trim();
        if !endpoint.is_empty() {
            config.endpoint.base_url = endpoint.to_string();
        }

        print!("Enter model name [gpt-oss-20b]: ");
        stdout.flush()?;
        let mut model = String::new();
        stdin.read_line(&mut model)?;
        let model = model.trim();
        if !model.is_empty() {
            config.endpoint.model = model.to_string();
        }

        println!(
            "Note: API keys are stored in plaintext in {:?}.",
            ChatConfig::config_path()?
        );
        print!("Enter API key (optional, press Enter to skip): ");
        stdout.flush()?;
        let mut api_key = String::new();
        stdin.read_line(&mut api_key)?;
        let api_key = api_key.trim();
        if !api_key.is_empty() {
            config.endpoint.api_key = Some(api_key.to_string());
        }

        // Test connection
        println!("\nTesting connection...");
        let client = LlmClient::new(config.endpoint.clone())?;

        match client.test_connection().await {
            Ok(_) => {
                println!("{} Connection successful!", "✓".green().bold());
                config.save()?;
                println!("Configuration saved to {:?}\n", ChatConfig::config_path()?);
                return Ok(true);
            }
            Err(e) => {
                eprintln!("{} Connection failed: {}", "✗".red().bold(), e);
                print!("\nRetry configuration? [y/N]: ");
                stdout.flush()?;
                let mut retry = String::new();
                stdin.read_line(&mut retry)?;
                if retry.trim().to_lowercase() != "y" {
                    return Ok(false);
                }
            }
        }
    }
}

/// Handle slash commands
async fn handle_slash_command(
    input: &str,
    config: &mut ChatConfig,
    approval_state: &mut ApprovalState,
    _client: &LlmClient,
) -> Result<()> {
    let parts: Vec<&str> = input[1..].split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }

    match parts[0] {
        "exit" | "quit" => {
            println!("Goodbye!");
            std::process::exit(0);
        }
        "config" => {
            if parts.len() == 1 {
                // Show config
                println!("\n{}", "Current Configuration:".bold());
                println!("  Endpoint: {}", config.endpoint.base_url.cyan());
                println!("  Model: {}", config.endpoint.model.cyan());
                println!(
                    "  API Key: {}",
                    if config.endpoint.api_key.is_some() {
                        "***"
                    } else {
                        "(none)"
                    }
                );
                println!("  Default Policy: {:?}", config.approvals.default_policy);
                println!("  Always Allow: {:?}", config.approvals.always_allow);
                println!();
            } else if parts.len() >= 3 && parts[1] == "set" {
                match parts[2] {
                    "endpoint" => {
                        if parts.len() < 4 {
                            eprintln!("Usage: /config set endpoint <url>");
                            return Ok(());
                        }
                        config.endpoint.base_url = parts[3].to_string();
                        config.save()?;
                        println!("{} Endpoint updated", "✓".green().bold());
                    }
                    "model" => {
                        if parts.len() < 4 {
                            eprintln!("Usage: /config set model <name>");
                            return Ok(());
                        }
                        config.endpoint.model = parts[3].to_string();
                        config.save()?;
                        println!("{} Model updated", "✓".green().bold());
                    }
                    _ => {
                        eprintln!("Unknown config key: {}", parts[2]);
                    }
                }
            }
        }
        "approvals" => {
            if parts.len() == 1 || parts[1] == "status" {
                println!("\n{}", "Approval Status:".bold());
                println!("  Default Policy: {:?}", config.approvals.default_policy);
                println!("  Always Allow: {:?}", config.approvals.always_allow);
                println!();
            } else if parts.len() >= 3 {
                match parts[1] {
                    "allow" => {
                        let name = parts[2];
                        config.add_always_allow(name.to_string());
                        config.save()?;
                        println!(
                            "{} Added {} to always-allow list",
                            "✓".green().bold(),
                            name.yellow()
                        );
                    }
                    "deny" => {
                        let name = parts[2];
                        config.remove_always_allow(name);
                        config.save()?;
                        println!(
                            "{} Removed {} from always-allow list",
                            "✓".green().bold(),
                            name.yellow()
                        );
                    }
                    "reset" => {
                        approval_state.reset();
                        println!("{} Reset session approvals", "✓".green().bold());
                    }
                    _ => {
                        eprintln!("Unknown approvals command: {}", parts[1]);
                    }
                }
            }
        }
        "help" => {
            println!("\n{}", "Chat Commands:".bold());
            println!("  /exit, /quit        Exit chat");
            println!("  /config             Show current configuration");
            println!("  /config set endpoint <url>");
            println!("  /config set model <name>");
            println!("  /approvals status   Show approval settings");
            println!("  /approvals allow <tool|category>");
            println!("  /approvals deny <tool|category>");
            println!("  /approvals reset    Reset session approvals");
            println!("  /help               Show this help");
            println!();
        }
        _ => {
            eprintln!("Unknown command: {}", parts[0]);
            eprintln!("Type /help for available commands");
        }
    }

    Ok(())
}

/// Prompt user for approval
fn prompt_approval(tool_name: &str, arguments: &serde_json::Value) -> Result<ApprovalDecision> {
    println!("\n{}", "Approval Required".yellow().bold());
    println!("Tool: {}", tool_name.cyan());
    println!("Arguments: {}", serde_json::to_string_pretty(arguments)?);
    println!("\nOptions:");
    println!("  1. Allow once");
    println!("  2. Always allow in this session");
    println!("  3. Always allow");
    println!("  4. Cancel");

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    print!("\nYour choice [1-4]: ");
    stdout.flush()?;

    let mut choice = String::new();
    stdin.read_line(&mut choice)?;

    match choice.trim() {
        "1" => Ok(ApprovalDecision::AllowOnce),
        "2" => Ok(ApprovalDecision::AllowSession),
        "3" => Ok(ApprovalDecision::AllowAlways),
        "4" | "" => Ok(ApprovalDecision::Cancel),
        _ => {
            eprintln!("Invalid choice, cancelling");
            Ok(ApprovalDecision::Cancel)
        }
    }
}
