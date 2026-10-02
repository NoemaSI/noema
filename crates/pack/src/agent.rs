//! The author agent: the rig runtime and its file-editing tools,
//! extracted from the recurve `agent-boot` crate (`rig_bridge` +
//! `factory::init_coder`) rather than depended on.
//!
//! Two agents share one runtime:
//!
//! - [`coder`]: the file-editing agent (read / write / edit / grep /
//!   glob / todo, sandboxed to one directory — for the evolution
//!   loop, the generation directory). It verifies contracts after
//!   editing; in the mechanism-IR loop it survives only as the
//!   *doctor* that repairs a mechanism.json the validator rejected.
//! - [`theorist`]: no tools, one turn. It authors mechanism ideas as
//!   JSON with all data inlined in the prompt; its output is parsed
//!   by [`crate::evolve`], never executed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use reloaded_code_core::{AllowedPathResolver, TodoState};
use reloaded_code_serdesai::{
    EditTool, GlobTool, GrepTool, ReadTool, SystemPromptBuilder, TodoReadTool, TodoWriteTool,
    WriteTool,
};
use rig::agent::{Agent, AgentBuilder, MultiTurnStreamItem, NoToolConfig, OutputMode};
use rig::client::{AgentModelExt, CompletionClient};
use rig::message::ImageMediaType;
use rig::message::Message;
use rig::message::ReasoningContent;
use rig::message::UserContent;
use rig::OneOrMany;
use rig::prelude::StreamingPrompt;
use rig::providers::openai::{self, GenericCompletionModel};
use rig::streaming::{StreamedAssistantContent, ToolCallDeltaContent};
use rig_agent::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use serde_json::{Value, json};
use serdes_ai::core::messages::ToolReturnContent;
use serdes_ai::tools::{RunContext, Tool as SerdesTool, ToolReturn};

/// Bridge a ReloadedCode (serdes-ai) tool into a rig dynamic tool.
/// Extracted verbatim from agent-boot's `rig_bridge`.
pub fn rig_tool<T, Deps>(tool: T) -> DynamicTool
where
    T: SerdesTool<Deps> + Send + Sync + 'static,
    Deps: Default + Send + Sync + 'static,
{
    let def = tool.definition();
    let name = def.name;
    let description = def.description;
    let parameters = def.parameters_json_schema;
    let tool = Arc::new(tool);

    DynamicTool::new(
        name,
        description,
        parameters,
        move |_ctx: &mut rig_agent::tool::ToolContext, args: Value| {
            let tool = Arc::clone(&tool);
            Box::pin(async move {
                let run_ctx = RunContext::new(Deps::default(), "rig");
                match tool.call(&run_ctx, args).await {
                    Ok(ToolReturn { content, .. }) => Ok(render(content)),
                    Err(e) => Err(ToolExecutionError::from_error(e)),
                }
            })
        },
    )
}

fn render(content: ToolReturnContent) -> ToolOutput {
    match content {
        ToolReturnContent::Text { content } => ToolOutput::text(content),
        ToolReturnContent::Json { content } => ToolOutput::json(content),
        ToolReturnContent::Error { error } => ToolOutput::text(error.message),
        ToolReturnContent::Image { .. } => ToolOutput::text("[tool returned image content]"),
        ToolReturnContent::Multiple { items } => match serde_json::to_value(&items) {
            Ok(v) => ToolOutput::json(v),
            Err(_) => ToolOutput::text("[tool returned multiple items]"),
        },
    }
}

/// Connection settings for the OpenAI-compatible completion endpoint
/// the author talks to.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl LlmConfig {
    /// `SKILL_*` env vars, falling back to `RWSI_*` (the
    /// self-improvement harness convention), falling back to the
    /// shipped llama.cpp defaults.
    pub fn from_env() -> Result<Self> {
        let pick = |name: &str, default: &str| -> String {
            std::env::var(format!("SKILL_{name}"))
                .or_else(|_| std::env::var(format!("RWSI_{name}")))
                .unwrap_or_else(|_| default.to_string())
        };

        Ok(Self {
            base_url: pick("BASE_URL", "http://192.168.88.192:9931/v1"),
            api_key: pick("API_KEY", "not-needed"),
            model: pick("MODEL", "glm-53-flash"),
        })
    }
}

impl From<&noema_config::LlmConfig> for LlmConfig {
    fn from(c: &noema_config::LlmConfig) -> Self {
        LlmConfig {
            base_url: c.endpoint.clone(),
            api_key: c.api_key.clone().unwrap_or_default(), // or fall back to env
            model: c.model_name.clone(),
        }
    }
}


/// Build the author agent: the domain preamble plus the
/// `SystemPromptBuilder` tool/working-directory prompt, with the file
/// tools sandboxed to `work_dir`.
pub fn coder(
    config: &LlmConfig,
    work_dir: &Path,
    preamble: &str,
) -> Result<Agent<GenericCompletionModel>> {
    let (tools, prompt_tail) = file_tools(work_dir, &[])?;

    let system_prompt = format!("{preamble}\n\n{prompt_tail}");

    Ok(reasoning_builder(&system_prompt, config)?
        .dynamic_tools(tools)
        .default_max_turns(140)
        .build())
}

/// The standard file tool set (read / write / edit / grep / glob /
/// todo) plus the `SystemPromptBuilder` tail it must be paired with.
/// Reading tools are sandboxed to `work_dir` plus `extra_reads`
/// (which may contain sensitive originals — see
/// [`crate::prepare`]'s read-only convention); writing tools stay
/// sandboxed to `work_dir` alone.
pub fn file_tools(
    work_dir: &Path,
    extra_reads: &[PathBuf],
) -> Result<(Vec<DynamicTool>, String)> {
    let work_dir: PathBuf = work_dir.to_path_buf();
    let working_dir_resolved = work_dir.display().to_string();

    let mut pb = SystemPromptBuilder::new().working_directory(&working_dir_resolved);

    let mut read_paths: Vec<String> = extra_reads
        .iter()
        .map(|path| path.display().to_string())
        .collect();

    read_paths.insert(0, working_dir_resolved.clone());

    let writes = AllowedPathResolver::new(vec![working_dir_resolved])?;
    let reads = AllowedPathResolver::new(read_paths)?;

    let todos = TodoState::new();

    let tools: Vec<DynamicTool> = vec![
        rig_tool::<_, ()>(pb.track(ReadTool::new(reads.clone()))),
        rig_tool::<_, ()>(pb.track(WriteTool::new(writes.clone()))),
        rig_tool::<_, ()>(pb.track(EditTool::new(writes.clone()))),
        rig_tool::<_, ()>(pb.track(GrepTool::new(reads.clone()))),
        rig_tool::<_, ()>(pb.track(GlobTool::new(reads.clone()))),
        rig_tool::<_, ()>(pb.track(TodoReadTool::new(todos.clone()))),
        rig_tool::<_, ()>(pb.track(TodoWriteTool::new(todos))),
    ];

    Ok((tools, pb.build()))
}

/// Build the theorist agent: the domain preamble only — no tools, one
/// turn. The theorist authors mechanism ideas as JSON; the filesystem
/// and the endpoint are invisible to it, so its only degree of
/// freedom is thinking, which is exactly the job.
///
/// The reply is constrained to the [`schemars::JsonSchema`] of `O`
/// through the endpoint's native structured-output mode (the
/// agent-boot extract pattern), so the envelope the caller parses is
/// produced by constrained decoding, not by the model volunteering
/// JSON in prose.
pub fn theorist<O>(config: &LlmConfig, preamble: &str) -> Result<Agent<GenericCompletionModel>>
where
    O: schemars::JsonSchema,
{
    let agent_builder = reasoning_builder(preamble, config)?
        .output_schema::<O>()
        .output_mode(OutputMode::Native);

    Ok(agent_builder.default_max_turns(1).build())
}

/// A tool-less, single-turn reasoning agent: one prompt in, one
/// answer out. For one-shot drafting where the caller validates and
/// re-prompts itself.
pub fn plain(config: &LlmConfig, preamble: &str) -> Result<Agent<GenericCompletionModel>> {
    Ok(reasoning_builder(preamble, config)?.default_max_turns(1).build())
}

/// The reasoning coder configuration (agent-boot's
/// `init_cenote_agent_reasoning_builder`): chat-completions endpoint,
/// thinking enabled.
pub(crate) fn reasoning_builder(
    system_prompt: &str,
    config: &LlmConfig,
) -> Result<AgentBuilder<GenericCompletionModel, NoToolConfig>> {
    let client = openai::Client::builder()
        .base_url(&config.base_url)
        .api_key(&config.api_key)
        .build()?;

    let thinking = json!({
        "chat_template_kwargs": { "enable_thinking": true },
        "reasoning_effort": "medium"
    });

    let builder = client
        .completion_model(&config.model)
        .completions_api()
        .into_agent_builder()
        .preamble(system_prompt)
        .record_content_telemetry(false)
        .additional_params(thinking);

    Ok(builder)
}

/// Forward one author-agent message to the SSE debug viewer
/// ([`crate::viewer`]); compiles to nothing without the `viewer`
/// feature.
#[cfg(feature = "viewer")]
fn publish(kind: &'static str, content: impl Into<String>) {
    crate::viewer::publish(kind, content);
}

#[cfg(not(feature = "viewer"))]
fn publish(_: &'static str, _: impl Into<String>) {}

/// Run one agent turn to completion, streaming every message to
/// stdout (and, under the `viewer` feature, to the SSE debug viewer),
/// and return the final response text. The tools execute inside the
/// rig runtime; file edits land on disk.
pub fn run(agent: &Agent<GenericCompletionModel>, prompt: &str) -> Result<String> {
    run_message(agent, prompt, None)
}

/// Run one agent turn with an optional image attachment (the
/// theorist's diagnostic plot) beside the text prompt. The viewer
/// receives the prompt text and the image as separate events, so the
/// browser renders the picture the model sees.
pub fn run_message(
    agent: &Agent<GenericCompletionModel>,
    text: &str,
    image_base64: Option<&str>,
) -> Result<String> {
    run_parts(agent, &[(text.to_string(), image_base64.map(str::to_string))])
}

/// Run one agent turn over interleaved text/image parts: each part's
/// text (and, when present, its image right after it) is appended to
/// the user message in order, so a prompt can place an image between
/// paragraphs — the referee sees the champion's trajectories beside
/// the champion's model and the candidate's beside the candidate's.
/// Streams to stdout and the SSE viewer like [`run`].
pub fn run_parts(
    agent: &Agent<GenericCompletionModel>,
    parts: &[(String, Option<String>)],
) -> Result<String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the tokio runtime for the author agent")?;

    let mut content = Vec::new();

    for (text, image) in parts {
        publish("prompt", text);

        if let Some(image) = image {
            publish("image", image.clone());

            content.push(UserContent::image_base64(
                image.as_str(),
                Some(ImageMediaType::PNG),
                None,
            ));
        }

        content.push(UserContent::text(text.clone()));
    }

    let prompt = Message::User {
        content: OneOrMany::many(content).expect("unable to construct the multimodal prompt"),
    };

    runtime.block_on(async {
        let mut stream = agent.stream_prompt(prompt).await;

        let mut response = String::new();

        while let Some(item) = stream.next().await {
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    publish("error", format!("{error}"));
                    return Err(anyhow::anyhow!("author agent stream failed: {error}"));
                }
            };

            match item {
                MultiTurnStreamItem::StreamAssistantItem(content) => match content {
                    StreamedAssistantContent::Text(delta) => {
                        publish("text", delta.text.clone());
                        print!("{}", delta.text);
                        let _ = std::io::stdout().flush();
                        response.push_str(&delta.text);
                    }
                    StreamedAssistantContent::ToolCallDelta { content, .. } => match content {
                        ToolCallDeltaContent::Name(name) => {
                            publish("tool_call", name.clone());
                            // println!("\n[dbg] tool call: {name}");
                        }
                        ToolCallDeltaContent::Delta(content) => {
                            publish("tool_call_delta", content.clone());
                            // println!("[dbg] tool call delta: {content}");
                        }
                    },
                    StreamedAssistantContent::ToolCall { tool_call, .. } => {
                        publish("tool_call", format!("{tool_call:#?}"));
                        // println!("[dbg] tool call: {tool_call:#?}");
                    }
                    StreamedAssistantContent::Final(out) => {
                        publish("usage", format!("{:#?}", out.usage));
                        // println!("[dbg] final: {:#?}", out.usage);
                    }
                    StreamedAssistantContent::Unknown(value) => {
                        publish("unknown", format!("{value:#?}"));
                        // println!("[dbg] unknown stream item: {value:#?}");
                    }
                    StreamedAssistantContent::Reasoning(reasoning) => {
                        let mut reasoning_text = Vec::new();

                        for message in reasoning.content {
                            match message {
                                ReasoningContent::Text { text, .. } => {
                                    reasoning_text.push(text)
                                }
                                ReasoningContent::Summary(text) => reasoning_text.push(text),
                                _ => {}//println!("[dbg] unknown reasoning message received"),
                            }
                        }

                        let joined = reasoning_text.join("\n");

                        publish("reasoning", joined.clone());
                        // println!("[dbg] reasoning: {joined}");
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        publish("reasoning_delta", reasoning.clone());
                        // println!("[dbg] reasoning delta: {reasoning}");
                    }
                },
                MultiTurnStreamItem::ToolExecutionCommitted {
                    internal_call_id,
                    tool_call,
                } => {
                    publish(
                        "tool_done",
                        format!("{tool_call:?} {internal_call_id} done"),
                    );
                    // println!("  [dbg] tool {tool_call:?} {internal_call_id} done");
                }
                MultiTurnStreamItem::FinalResponse(final_response) => {
                    let usage = final_response.usage();

                    publish(
                        "usage",
                        format!("{} in / {} out tokens", usage.input_tokens, usage.output_tokens),
                    );

                    println!(
                        "\n[dbg] {} in / {} out tokens",
                        usage.input_tokens, usage.output_tokens,
                    );
                }
                _ => {
                    publish("unknown", "unhandled stream item");
                    // println!("[dbg] unhandled stream item");
                }
            }
        }

        println!();

        Ok(response)
    })
}
