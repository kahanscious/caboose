use std::io::{IsTerminal, Read as _};

use anyhow::Result;
use clap::Parser;

mod agent;
mod app;
mod attachment;
mod checkpoint;
mod circuits;
mod clipboard;
mod config;
mod hooks;
mod init;
mod lsp;
mod mcp;
mod memory;
mod migrate;
mod provider;
mod roundhouse;
mod safety;
mod scm;
mod session;
mod skills;
mod sub_agent;
mod terminal;
mod tools;
mod tui;
mod update;

/// Caboose — a terminal-native AI coding agent
#[derive(Parser, Debug)]
#[command(name = "caboose", version, about)]
struct Cli {
    /// Run a single prompt non-interactively and exit
    #[arg(short, long)]
    prompt: Option<String>,

    /// Model to use (e.g. "claude-sonnet-4-20250514", "gpt-4o")
    #[arg(short, long)]
    model: Option<String>,

    /// Provider to use (e.g. "anthropic", "openai")
    #[arg(long)]
    provider: Option<String>,

    /// Resume a previous session by ID
    #[arg(short, long)]
    session: Option<String>,

    /// Working directory (defaults to cwd)
    #[arg(short = 'd', long)]
    cwd: Option<String>,

    /// Permission mode: plan, default, auto-edit, chug
    #[arg(long, default_value = "default")]
    mode: String,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Update caboose to the latest version
    Update {
        /// Just check for updates, don't install
        #[arg(long)]
        check: bool,
    },
    /// Run the background circuit daemon
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
}

#[derive(clap::Subcommand, Debug)]
enum DaemonAction {
    /// Start the daemon
    Start,
    /// Stop the daemon
    Stop,
    /// List active circuits
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing — suppress logs to stderr in non-interactive mode
    // unless debug is enabled
    let is_interactive = cli.prompt.is_none() && std::io::stdin().is_terminal();
    let filter = if cli.debug {
        "debug"
    } else if is_interactive {
        // In TUI mode, suppress rmcp/hyper/reqwest info logs — they bleed
        // through crossterm's alternate screen as raw text.
        "info,rmcp=warn,hyper=warn,reqwest=warn"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    // Load config
    let config = config::Config::load()?;

    // Set working directory if specified
    if let Some(ref cwd) = cli.cwd {
        std::env::set_current_dir(cwd)?;
    }

    // Handle subcommands
    if let Some(command) = cli.command {
        match command {
            Command::Update { check } => {
                return update::run(check).await;
            }
            Command::Daemon { action } => {
                match action {
                    Some(DaemonAction::Start) | None => {
                        if circuits::daemon::is_daemon_running() {
                            eprintln!("daemon is already running");
                            std::process::exit(1);
                        }
                        return circuits::daemon::run_daemon().await;
                    }
                    Some(DaemonAction::Stop) => {
                        if !circuits::daemon::is_daemon_running() {
                            eprintln!("no daemon running");
                            std::process::exit(1);
                        }
                        if let Some(port) = circuits::ipc::read_daemon_port() {
                            match tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await
                            {
                                Ok(mut stream) => {
                                    use circuits::ipc::*;
                                    let _ =
                                        send_message(&mut stream, &DaemonRequest::Shutdown).await;
                                    eprintln!("daemon stop requested");
                                }
                                Err(e) => {
                                    eprintln!("could not connect to daemon: {e}");
                                    // Stale lockfile — clean it up
                                    let _ = circuits::daemon::remove_lockfile();
                                }
                            }
                        } else {
                            eprintln!("could not read daemon port from lockfile");
                        }
                        return Ok(());
                    }
                    Some(DaemonAction::List) => {
                        if !circuits::daemon::is_daemon_running() {
                            eprintln!("no daemon running");
                            std::process::exit(1);
                        }
                        if let Some(port) = circuits::ipc::read_daemon_port() {
                            match tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await
                            {
                                Ok(mut stream) => {
                                    use circuits::ipc::*;
                                    let _ = send_message(&mut stream, &DaemonRequest::ListCircuits)
                                        .await;
                                    let mut reader = tokio::io::BufReader::new(&mut stream);
                                    match read_message::<DaemonResponse>(&mut reader).await {
                                        Ok(DaemonResponse::CircuitList(circuits)) => {
                                            if circuits.is_empty() {
                                                println!("no circuits");
                                            } else {
                                                for c in &circuits {
                                                    println!(
                                                        "{} | {:?} | {} | {}s",
                                                        c.id, c.status, c.prompt, c.interval_secs
                                                    );
                                                }
                                            }
                                        }
                                        Ok(DaemonResponse::Error(e)) => {
                                            eprintln!("daemon error: {e}")
                                        }
                                        Ok(_) => eprintln!("unexpected response"),
                                        Err(e) => eprintln!("failed to read response: {e}"),
                                    }
                                }
                                Err(e) => {
                                    eprintln!("could not connect to daemon: {e}");
                                    let _ = circuits::daemon::remove_lockfile();
                                }
                            }
                        } else {
                            eprintln!("could not read daemon port from lockfile");
                        }
                        return Ok(());
                    }
                }
            }
        }
    }

    // Determine prompt: --prompt flag or piped stdin
    let prompt = if let Some(p) = cli.prompt {
        Some(p)
    } else if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        let trimmed = buf.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        None
    };

    if let Some(prompt) = prompt {
        return run_non_interactive(config, prompt, cli.model, cli.provider, cli.mode).await;
    }

    // Install panic hook that restores terminal before printing panic info
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort terminal restore so the panic message is readable
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableBracketedPaste,
        );
        let _ = crossterm::terminal::disable_raw_mode();
        default_hook(info);
    }));

    // Launch the TUI app
    let mut app = app::App::new(config, cli.model, cli.provider, cli.session, cli.mode).await?;
    app.connect_mcp_servers().await;
    app.run().await
}

/// Run a single prompt through the agent loop without a TUI.
/// Prints the final assistant response to stdout and exits.
async fn run_non_interactive(
    mut config: config::Config,
    prompt: String,
    model: Option<String>,
    provider_name: Option<String>,
    mode: String,
) -> Result<()> {
    let providers = provider::ProviderRegistry::new(&config);
    let provider = providers.get_provider(provider_name.as_deref(), model.as_deref())?;

    let system_prompt = config
        .system_prompt
        .clone()
        .unwrap_or_else(|| {
            "You are a helpful AI coding assistant. You have access to tools for reading, \
             writing, and searching files, running shell commands, and fetching URLs. \
             Use them to help the user with their coding tasks.\n\n\
             Use `glob` and `grep` to locate relevant files before reading them — don't guess paths. \
             Use `read_file` with `offset`/`limit` for targeted reads. Read a small window first \
             (50–100 lines) to orient, then target specific sections. \
             Batch independent tool calls in a single response — multiple reads, greps, or globs \
             can run in one turn. \
             Don't re-read files already in context unless they've been modified. \
             When `read_file` output is truncated, use `offset`/`limit` to read the specific section \
             you need rather than increasing the limit.\n\n\
             You have `todo_write` and `todo_read` tools for task management. \
             Use `todo_write` for multi-step tasks (3+ steps) to show progress. \
             Use `todo_read` to check current task state before updating. \
             Each `todo_write` call replaces the entire task list. Keep task names short. \
             Mark tasks completed immediately after finishing each one. \
             Mark tasks cancelled if they are no longer needed. \
             Statuses: pending, in_progress, completed, cancelled."
                .to_string()
        });

    let caboose_md = std::fs::read_to_string("CABOOSE.md").ok();
    let system_prompt =
        crate::init::handler::inject_caboose_md(system_prompt, caboose_md.as_deref());

    let permission_mode = agent::permission::PermissionMode::from_str_loose(&mode);
    let mut agent = agent::AgentLoop::new(system_prompt, permission_mode);

    // Wire tools config
    if let Some(ref tools_cfg) = config.tools {
        if let Some(ref allow) = tools_cfg.allow_commands {
            agent.allow_list = allow.clone();
        }
        if let Some(ref deny) = tools_cfg.deny_commands {
            agent.deny_list = deny.clone();
        }
        if let Some(ref secrets) = tools_cfg.additional_secret_names {
            agent.additional_secrets = secrets.clone();
        }
    }

    // Discover schemas for executable tools that lack description/args
    if let Some(ref mut tools_cfg) = config.tools
        && let Some(ref exec_tools) = tools_cfg.executable
    {
        let discovered = crate::tools::executable::discover_all(exec_tools).await;
        tools_cfg.executable = Some(discovered);
    }

    let cli_tools_ref = config.tools.as_ref().and_then(|t| t.registry.as_ref());
    let exec_tools_ref = config.tools.as_ref().and_then(|t| t.executable.as_ref());
    let headless_cwd = std::env::current_dir().unwrap_or_default();
    let headless_scm = crate::scm::detection::detect_provider(&headless_cwd);
    let tool_registry = tools::ToolRegistry::new(cli_tools_ref, exec_tools_ref, &headless_scm);
    let mcp_config = config.mcp.clone().unwrap_or_default();
    let mut mcp_manager = crate::mcp::McpManager::from_config(&mcp_config);
    mcp_manager.connect_all().await;
    let mut tool_defs = tool_registry.definitions().to_vec();
    tool_defs.extend(mcp_manager.tool_definitions());

    // Fire SessionStart hooks
    if let Some(ref hooks_config) = config.hooks
        && !hooks_config.session_start.is_empty()
    {
        let context = serde_json::json!({
            "event": "SessionStart",
            "session_id": "",
            "resumed": false,
        });
        let _ = crate::hooks::fire_hooks(&hooks_config.session_start, context).await;
    }

    // Start the agent with the user's prompt
    agent.send_message(prompt, provider.as_ref(), &tool_defs);

    // Poll loop — drive the agent to completion
    loop {
        // Small delay to avoid busy-spinning
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let events = agent.poll();
        for event in &events {
            match event {
                agent::AgentEvent::Error(e) => {
                    eprintln!("Error: {e}");
                    return Err(anyhow::anyhow!("{e}"));
                }
                agent::AgentEvent::CompactionComplete => {
                    // Resume stream after compaction if needed
                    if !agent.stashed_tool_defs.is_empty() {
                        let defs: Vec<_> = std::mem::take(&mut agent.stashed_tool_defs);
                        agent.start_stream(provider.as_ref(), &defs);
                    }
                }
                _ => {}
            }
        }

        match agent.state {
            agent::AgentState::Idle => break,
            agent::AgentState::Streaming | agent::AgentState::Compacting => {
                // Keep polling
            }
            agent::AgentState::PendingApproval { .. } => {
                // In non-interactive mode, auto-approve all tools
                while agent.approve_current() {}
            }
            agent::AgentState::ExecutingTools => {
                let results = agent
                    .execute_pending_tools(
                        &mut mcp_manager,
                        config.services.as_ref(),
                        cli_tools_ref,
                        config
                            .tools
                            .as_ref()
                            .and_then(|t| t.deny_commands.as_deref())
                            .unwrap_or(&[]),
                        config.hooks.as_ref(),
                        exec_tools_ref,
                    )
                    .await;
                // Track tool results for debugging
                for r in &results {
                    if r.is_error {
                        eprintln!(
                            "Tool {} error: {}",
                            r.tool_name.as_deref().unwrap_or("unknown"),
                            r.output
                        );
                    }
                }
                agent.continue_after_tools(provider.as_ref(), &tool_defs);
            }
        }
    }

    // Fire SessionEnd hooks
    if let Some(ref hooks_config) = config.hooks
        && !hooks_config.session_end.is_empty()
    {
        let context = serde_json::json!({
            "event": "SessionEnd",
            "session_id": "",
            "message_count": agent.conversation.messages.len(),
        });
        let _ = crate::hooks::fire_hooks(&hooks_config.session_end, context).await;
    }

    // Extract the last assistant message from conversation
    let last_text = agent
        .conversation
        .messages
        .iter()
        .rev()
        .find_map(|msg| {
            if matches!(msg.role, agent::conversation::Role::Assistant) {
                match &msg.content {
                    agent::conversation::Content::Text(t) => Some(t.clone()),
                    agent::conversation::Content::Blocks(blocks) => blocks.iter().find_map(|b| {
                        if let agent::conversation::ContentBlock::Text { text } = b {
                            Some(text.clone())
                        } else {
                            None
                        }
                    }),
                }
            } else {
                None
            }
        })
        .unwrap_or_default();

    println!("{last_text}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parse_no_args_defaults_to_no_subcommand() {
        let cli = Cli::try_parse_from(["caboose"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_update_subcommand() {
        let cli = Cli::try_parse_from(["caboose", "update"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(super::Command::Update { check: false })
        ));
    }

    #[test]
    fn parse_update_check_flag() {
        let cli = Cli::try_parse_from(["caboose", "update", "--check"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(super::Command::Update { check: true })
        ));
    }

    #[test]
    fn parse_prompt_flag_still_works() {
        let cli = Cli::try_parse_from(["caboose", "-p", "hello"]).unwrap();
        assert_eq!(cli.prompt.as_deref(), Some("hello"));
        assert!(cli.command.is_none());
    }
}
