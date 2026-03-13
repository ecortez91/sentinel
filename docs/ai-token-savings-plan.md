# AI Token Savings & Multi-Provider Plan

> **Status**: Planning — not yet implemented  
> **Created**: 2026-03-12  
> **Context**: All 23 features complete (Phases 1-8 committed). This is the next major initiative.

---

## Problem Statement

Sentinel's AI integration burns tokens unnecessarily:

- **Auto-analysis** runs every 5 minutes by default, sending full system context (~3-5K input tokens) to `claude-opus-4-6` with a 4096 max output — even when the system is idle and nobody is looking at the dashboard.
- **One model for everything** — Opus is used for the dashboard summary card, market sentiment (200 words), and command palette fallback. Haiku or Sonnet would suffice for all of these.
- **No token tracking** — users have zero visibility into consumption.
- **No provider choice** — only Claude is supported. Users with OpenAI keys or local Ollama instances can't use Sentinel's AI features.
- **Command palette typos trigger AI calls** — any unrecognized input (including typos like `:wh`) falls through to Claude.
- **No idle detection** — auto-analysis fires even when CPU is flat and nothing has changed.

---

## Current Architecture (as of Phase 8)

### 5 AI Call Sites

| # | Function | Trigger | Model | Max Tokens | Frequency |
|---|----------|---------|-------|-----------|-----------|
| 1 | `dispatch_ai_chat()` | User sends chat message (AskAi tab) | opus | 4096 | On user action |
| 2 | `dispatch_ai_chat()` | User presses `a` on process (Processes tab) | opus | 4096 | On user action |
| 3 | `dispatch_insight()` | Auto-analysis timer (default 300s) | opus | 4096 | Automatic |
| 4 | `dispatch_command_ai()` | Unknown command palette input | opus | 4096 | On typo/query |
| 5 | `dispatch_plugin_ai()` | User presses `a` in Market detail | opus | 4096 | On user action |

### Key Files

| File | Role |
|------|------|
| `src/ai/client.rs` | Auth discovery, token refresh, API requests, SSE streaming (389 lines) |
| `src/ai/context.rs` | Builds rich system context string for the LLM (346 lines) |
| `src/ai/conversation.rs` | Conversation history with rolling window (266 lines) |
| `src/constants.rs` | All AI constants — model, max_tokens, context limits (lines 274-300) |
| `src/config/mod.rs` | Config — currently NO AI-specific fields |
| `src/app.rs` | App struct, all 4 dispatch functions, event loop drain functions |

### Auth Methods (Priority Order)

1. `ANTHROPIC_API_KEY` env var (detects OAuth token by `sk-ant-oat` prefix)
2. OpenCode `~/.local/share/opencode/auth.json` (with auto token refresh)
3. Claude Code `~/.claude/.credentials.json`

### What's Provider-Agnostic Already

- `AiEvent` enum (Chunk/Done/Error) — no Anthropic-specific data
- `ContextBuilder::build()` — produces plain text, no API coupling
- `Conversation` — stores messages as generic role+content pairs
- `model: String` field on `ClaudeClient` — already a runtime value

---

## Proposed Changes

### Phase A: Provider Abstraction Layer

**Goal**: Extract a `trait AiProvider` so we can plug in Claude, OpenAI, and Ollama behind a common interface.

**New files:**

1. **`src/ai/provider.rs`** — The core trait:
   ```rust
   #[async_trait]
   pub trait AiProvider: Send + Sync {
       fn name(&self) -> &str;                    // "Claude", "OpenAI", "Ollama"
       fn supports_streaming(&self) -> bool;
       async fn ask_streaming(
           &self,
           system_prompt: &str,
           messages: &[ApiMessage],
           model: &str,
           max_tokens: u32,
           tx: mpsc::UnboundedSender<AiEvent>,
       );
   }
   ```

2. **`src/ai/claude.rs`** — Rename from `client.rs`. Refactor `ClaudeClient` to implement `AiProvider`. Keep all Anthropic-specific logic here (SSE parsing, OAuth refresh, beta headers, `anthropic-version` header). Keep existing auth discovery.

3. **`src/ai/openai.rs`** — `OpenAiProvider` implementing `AiProvider`:
   - Endpoint: `https://api.openai.com/v1/chat/completions` (or configurable for Azure)
   - Auth: `OPENAI_API_KEY` env var
   - SSE format: OpenAI's `data: {"choices":[{"delta":{"content":"..."}}]}`
   - Model mapping: `gpt-4o` for premium, `gpt-4o-mini` for cheap tier

4. **`src/ai/ollama.rs`** — `OllamaProvider` implementing `AiProvider`:
   - Endpoint: `http://localhost:11434/api/chat` (configurable)
   - No auth required
   - Streaming: Ollama's newline-delimited JSON
   - Model: configurable (auto-detect installed models via `/api/tags`)
   - **Zero cost** — runs locally

5. **`src/ai/mod.rs`** — Update exports. Add factory function:
   ```rust
   pub fn create_provider(config: &AiConfig) -> Option<Box<dyn AiProvider>>
   ```

6. **`src/app.rs`** — Replace `claude_client: Option<ClaudeClient>` with `ai_provider: Option<Box<dyn AiProvider>>`. Update all 4 dispatch functions to use the trait. Stop re-discovering auth per request — use the stored provider.

**Dependencies**: May need `async-trait` crate.

---

### Phase B: Model Tiering

**Goal**: Use cheap models for auto-tasks, premium models for interactive chat.

**Modifications to `src/constants.rs`:**

```rust
// Premium tier — interactive chat, process questions
CLAUDE_MODEL_PREMIUM    = "claude-opus-4-6"
OPENAI_MODEL_PREMIUM    = "gpt-4o"

// Cheap tier — auto-analysis, market sentiment, command palette
CLAUDE_MODEL_CHEAP      = "claude-haiku-4-20250414"
OPENAI_MODEL_CHEAP      = "gpt-4o-mini"

// Ollama — user-configured, single tier
OLLAMA_MODEL_DEFAULT    = "llama3.1"

// Per-call-site max_tokens
CHAT_MAX_TOKENS              = 4096   // interactive chat (keep)
AUTO_ANALYSIS_MAX_TOKENS     = 1024   // down from 4096
MARKET_AI_MAX_TOKENS         = 512    // market sentiment
COMMAND_AI_MAX_TOKENS        = 1024   // command palette fallback
```

**Modifications to `src/app.rs`** — Each dispatch passes the right model + max_tokens:

| Dispatch | Model Tier | Max Tokens |
|----------|-----------|-----------|
| `dispatch_ai_chat()` | Premium | 4096 |
| `dispatch_insight()` | Cheap | 1024 |
| `dispatch_command_ai()` | Cheap | 1024 |
| `dispatch_plugin_ai()` | Cheap | 512 |

---

### Phase C: AI Configuration

**Goal**: Users configure their provider and models in `config.toml`.

**Add to `src/config/mod.rs`:**

```rust
pub struct AiConfig {
    pub provider: AiProviderType,        // Claude, OpenAI, Ollama
    pub premium_model: String,           // override for interactive chat
    pub cheap_model: String,             // override for auto-tasks
    pub ollama_url: String,              // default: http://localhost:11434
    pub openai_url: String,              // default: https://api.openai.com (Azure override)
    pub auto_analysis_enabled: bool,     // default: false (opt-in)
    pub auto_analysis_interval_secs: u64, // default: 600
    pub token_budget_per_hour: Option<u64>, // None = unlimited
}

pub enum AiProviderType {
    Claude,
    OpenAI,
    Ollama,
}
```

Plus the corresponding `FileAiConfig`, `WriteAiConfig`, merge logic, and `From` impls (same pattern as `WindowsConfig`, `SecurityConfig`, etc.).

**Environment variable fallbacks:**
- `SENTINEL_AI_PROVIDER` — overrides `provider`
- `OPENAI_API_KEY` — auth for OpenAI provider
- `OLLAMA_URL` — overrides `ollama_url`
- Existing `ANTHROPIC_API_KEY` — auth for Claude provider

**Example `config.toml`:**
```toml
[ai]
provider = "ollama"          # "claude", "openai", "ollama"
premium_model = "llama3.1"   # used for chat
cheap_model = "llama3.1"     # used for auto-analysis
ollama_url = "http://localhost:11434"
auto_analysis_enabled = true
auto_analysis_interval_secs = 600
# token_budget_per_hour = 50000  # uncomment to cap spending
```

---

### Phase D: Token Tracking & Display

**Goal**: Parse token usage from API responses, track it, show it to the user.

**Add `AiEvent::Usage` variant:**

```rust
pub enum AiEvent {
    Chunk(String),
    Done,
    Error(String),
    Usage { input_tokens: u64, output_tokens: u64 },  // NEW
}
```

**Provider-specific parsing:**
- **Claude**: Parse `message_start` SSE event -> `usage.input_tokens`. Parse `message_delta` -> `usage.output_tokens`.
- **OpenAI**: Parse `usage` field in final chunk.
- **Ollama**: Parse `eval_count` / `prompt_eval_count` from response.

**Add to `src/ui/state.rs`:**

```rust
// -- AI token tracking --
pub ai_tokens_input: u64,     // session total input tokens
pub ai_tokens_output: u64,    // session total output tokens
pub ai_call_count: u32,       // session total API calls
```

**Display**: Show in the status bar: `"AI: 12.3K tokens | 7 calls"` (or in Settings "AI" category as read-only).

**Budget enforcement** (if `token_budget_per_hour` is set):
- Track per-hour rolling window
- Stop AI calls when budget exceeded
- Show warning in status bar: `"AI: budget exceeded (50K/hr)"`

---

### Phase E: Smart Auto-Analysis

**Goal**: Don't waste tokens when nothing has changed or nobody is looking.

**Modifications to `src/app.rs` `tick_auto_analysis()`:**

```rust
fn tick_auto_analysis(&mut self) {
    if !self.auto_analysis_enabled { return; }
    if !self.has_key { return; }
    if self.state.ai_insight_loading { return; }

    // NEW: Skip if user isn't on Dashboard tab
    if self.state.active_tab != Tab::Dashboard { return; }

    // NEW: Skip if system is idle since last analysis
    if self.is_system_idle_since_last_analysis() { return; }

    // NEW: Skip if token budget exceeded
    if self.is_token_budget_exceeded() { return; }

    // ... existing timer logic ...
}
```

**`is_system_idle_since_last_analysis()`** checks:
- CPU usage delta < 5% since last analysis
- No new alerts since last analysis
- No significant memory change (> 10%)
- If all idle -> skip, saves the call

**Default changed**: `DEFAULT_AUTO_ANALYSIS_ENABLED = false` (opt-in only).

**Default interval changed**: `DEFAULT_AUTO_ANALYSIS_SECS = 600` (10 min, up from 5 min).

---

### Phase F: Command Palette AI Guard

**Goal**: Prevent accidental AI calls from typos.

**Modifications to `src/app.rs` `execute_command()`** — before the `_ =>` fallthrough:

```rust
// Don't send to AI if input is too short (likely a typo)
if input.len() < 5 {
    return CommandResult::text_only(
        format!("Unknown command: '{}'. Type :help for available commands.", input)
    );
}

// Don't send if it looks like a partial known command
let known = ["why", "slow", "timeline", "port", "pid", "disk", "anomaly", "help", ...];
if known.iter().any(|c| c.starts_with(input) || input.starts_with(&c[..2.min(c.len())])) {
    let matches: Vec<_> = known.iter()
        .filter(|c| c.starts_with(&input[..2.min(input.len())]))
        .collect();
    return CommandResult::text_only(
        format!("Unknown command. Did you mean: {}?", matches.join(", "))
    );
}
```

---

### Phase G: Context & History Reduction

**Goal**: Reduce token bloat.

**1. Reduce conversation history** (`src/constants.rs`):
```rust
MAX_CONVERSATION_HISTORY = 20   // down from 50
```

**2. Add light context builder** (`src/ai/context.rs`):

New `ContextBuilder::build_light()` for auto-analysis / command palette:

| Section | Full (`build()`) | Light (`build_light()`) |
|---------|-----------------|------------------------|
| Top CPU processes | 25 | 10 |
| Top memory processes | 15 | 5 |
| Process groups | 20 | 5 |
| Alerts | 30 | 10 |
| Network interfaces | 10 | Skip |
| Developer processes | All | Skip |
| Filesystems | All | All (small) |

This cuts auto-analysis input tokens by ~50-60%.

**3. Update dispatch functions** (`src/app.rs`):
- `dispatch_ai_chat()` -> `ContextBuilder::build()` (full)
- `dispatch_insight()` -> `ContextBuilder::build_light()` (light)
- `dispatch_command_ai()` -> `ContextBuilder::build_light()` (light)
- `dispatch_plugin_ai()` -> no system context (already plugin-specific)

---

## Estimated Token Savings

| Change | Savings | Phase |
|--------|---------|-------|
| Auto-analysis off by default | ~100% for non-opt-in users | E |
| Haiku/mini for auto-analysis | ~90% cost per call | B |
| Auto-analysis idle skip | ~70% fewer calls when enabled | E |
| Reduced auto-analysis max_tokens (4096->1024) | ~75% output tokens | B |
| Light context for auto-analysis | ~50-60% input tokens | G |
| Haiku/mini for market & command AI | ~90% cost per call | B |
| Command palette typo guard | Eliminates accidental calls | F |
| Conversation history 50->20 | Smaller context over time | G |
| Ollama provider option | 100% savings (free/local) | A |

**Combined effect for a user who enables auto-analysis with Haiku**:
- Before: ~1.4M input tokens/day (Opus, 5-min interval, full context, always firing)
- After: ~15K input tokens/day (Haiku, 10-min interval, light context, idle skip, dashboard-only)
- **~99% reduction**

---

## Implementation Order

| Order | Phase | Description | Effort |
|-------|-------|-------------|--------|
| 1 | **A** | Provider abstraction layer | Heavy (~60% of total) |
| 2 | **C** | AI configuration in config.toml | Medium (with A) |
| 3 | **B** | Model tiering | Quick (once A done) |
| 4 | **E** | Smart auto-analysis guards | Medium |
| 5 | **D** | Token tracking & display | Medium |
| 6 | **F** | Command palette typo guard | Quick |
| 7 | **G** | Context & history reduction | Quick |

**Total estimate**: ~3-4 sessions of work.

---

## Open Questions

1. **Ollama model auto-detection** — Should we query `/api/tags` to list installed models and let the user pick from the Settings UI, or just default to `llama3.1` and let them configure in `config.toml`?

2. **Azure OpenAI** — Some enterprise users use Azure OpenAI with custom endpoints. Support a configurable `openai_url` field, or defer to a later phase?

3. **Settings UI for AI** — Should the Settings plugin get a new "AI" category (provider picker, model selector, token budget, auto-analysis toggle), or is `config.toml` + env vars enough?

4. **Streaming differences** — Ollama and OpenAI have slightly different streaming formats. Need to handle edge cases (Ollama sometimes sends empty deltas, OpenAI sends `[DONE]` differently than Claude).

5. **Provider hot-switching** — Can users change providers at runtime via Settings, or only via config + restart?

---

## Dependencies

- `async-trait` crate (for `#[async_trait]` on the provider trait)
- No new HTTP client needed — `reqwest` already handles all three providers
- `serde_json` already available for parsing all response formats
