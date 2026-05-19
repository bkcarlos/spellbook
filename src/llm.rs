//! LLM client + manager.
//!
//! - Two providers: OpenAI-compatible (covers OpenAI / DeepSeek / Ollama-with-/v1) and Anthropic.
//! - All HTTP is blocking on a background thread; results come back over an mpsc channel
//!   that the UI loop polls each frame.
//! - Built-in throttle (sliding window) and 24h same-command cache.
//! - Blocklist for dangerous commands (no LLM hallucinating about `rm -rf /`).

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

// ============================================================================
// Config
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Provider {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "anthropic")]
    Anthropic,
}

impl Provider {
    pub fn label(&self) -> &'static str {
        match self {
            Provider::OpenAi => "OpenAI compatible",
            Provider::Anthropic => "Anthropic",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: Provider,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    pub timeout_secs: u64,
    pub max_tokens: u32,
    pub offline_mode: bool,
    pub rate_per_minute: u32,
    pub features: LlmFeatures,
    pub consent: Consent,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: Provider::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            timeout_secs: 12,
            max_tokens: 800,
            offline_mode: false,
            rate_per_minute: 30,
            features: LlmFeatures::default(),
            consent: Consent::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmFeatures {
    pub paste_enhance: bool,
    pub generate_description: bool,
    pub generate_command: bool,
    pub explain_command: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Consent {
    pub paste_enhance: Option<DateTime<Utc>>,
}

fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("no config dir")?;
    let p = base.join("Spellbook");
    std::fs::create_dir_all(&p).ok();
    Ok(p)
}

fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("llm.toml"))
}

impl LlmConfig {
    pub fn load_or_default() -> Self {
        match config_path().ok().and_then(|p| std::fs::read_to_string(&p).ok()) {
            Some(s) => toml::from_str(&s).unwrap_or_default(),
            None => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        let s = toml::to_string_pretty(self)?;
        std::fs::write(&path, s).context("write llm config")?;
        Ok(())
    }

    pub fn is_local(&self) -> bool {
        self.base_url.contains("localhost") || self.base_url.contains("127.0.0.1")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeySource {
    Env,
    Keychain,
    None,
}

// ============================================================================
// Keychain helpers — wrapped so backend failures (e.g. no Secret Service on
// a headless Linux box) are silent rather than crashing the app.
// ============================================================================

const KEYRING_SERVICE: &str = "Spellbook";

fn keyring_get(env_name: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, env_name).ok()?;
    entry.get_password().ok()
}

pub fn keyring_set(env_name: &str, key: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, env_name)
        .map_err(|e| anyhow!("keychain entry: {e}"))?;
    entry
        .set_password(key)
        .map_err(|e| anyhow!("keychain write: {e}"))?;
    Ok(())
}

pub fn keyring_delete(env_name: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, env_name)
        .map_err(|e| anyhow!("keychain entry: {e}"))?;
    match entry.delete_credential() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()), // already gone
        Err(e) => Err(anyhow!("keychain delete: {e}")),
    }
}

// ============================================================================
// Provider-level HTTP
// ============================================================================

fn complete(config: &LlmConfig, api_key: &str, system: &str, user: &str) -> Result<String> {
    if config.offline_mode {
        bail!("offline mode");
    }
    match config.provider {
        Provider::OpenAi => complete_openai(config, api_key, system, user),
        Provider::Anthropic => complete_anthropic(config, api_key, system, user),
    }
}

fn complete_openai(config: &LlmConfig, api_key: &str, system: &str, user: &str) -> Result<String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "max_tokens": config.max_tokens,
        "temperature": 0.2,
        "stream": false,
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(config.timeout_secs))
        .build();
    let mut req = agent.post(&url).set("Content-Type", "application/json");
    if !api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {api_key}"));
    }
    let resp = req
        .send_json(body)
        .map_err(|e| anyhow!("http error: {e}"))?;
    let json: serde_json::Value = resp.into_json().context("parse json")?;
    let text = json
        .get("choices")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("unexpected response shape"))?;
    Ok(text.to_string())
}

fn complete_anthropic(
    config: &LlmConfig,
    api_key: &str,
    system: &str,
    user: &str,
) -> Result<String> {
    let url = format!("{}/messages", config.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(config.timeout_secs))
        .build();
    let resp = agent
        .post(&url)
        .set("x-api-key", api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow!("http error: {e}"))?;
    let json: serde_json::Value = resp.into_json().context("parse json")?;
    let text = json
        .get("content")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("unexpected response shape"))?;
    Ok(text.to_string())
}

// ============================================================================
// Prompts
// ============================================================================

const SYS_ENHANCE: &str = "你是一个开发者命令笔记助手。根据给定的 shell 命令和规则推断的初始结果，生成更高质量的 title / description / tags。严格只输出 JSON。";

const SYS_DESCRIBE: &str = "你是一个命令文档助手。为给定的 shell 命令生成 Markdown 格式的说明，包含简述和逐参数解释。只输出 Markdown，不要任何额外文字。";

const SYS_GENERATE: &str = "你是一个 shell 命令生成助手。根据用户的自然语言描述生成对应的命令。严格只输出 JSON。";

const SYS_EXPLAIN: &str = "你是一个 shell 命令解释助手。逐参数解释命令的作用，列出注意事项。Markdown 格式，简洁。只输出 Markdown，不要任何额外文字。";

fn prompt_enhance(command: &str, rule_title: &str, rule_category: &str, rule_tags: &str) -> String {
    format!(
        r#"为这条 shell 命令生成 JSON：
{{
  "title": "简短中文标题，不超过 20 字",
  "description": "Markdown 说明，含简述和逐参数解释",
  "tags": ["英文小写标签", "..."]
}}

命令：
{command}

规则推断（参考，可改可不改）：
- title:    {rule_title}
- category: {rule_category}
- tags:     {rule_tags}

只输出 JSON，无 markdown 代码块包裹。"#
    )
}

fn prompt_describe(title: &str, command: &str) -> String {
    format!(
        r#"标题：{title}
命令：{command}

请输出 Markdown 说明（含简述与逐参数解释）。"#
    )
}

fn prompt_generate(user_prompt: &str) -> String {
    format!(
        r#"用户描述：
{user_prompt}

输出 JSON：
{{
  "command": "生成的命令（单行或多行）",
  "description": "Markdown 说明"
}}

只输出 JSON。"#
    )
}

fn prompt_explain(command: &str) -> String {
    format!(
        r#"命令：
{command}

请输出 Markdown 解释。"#
    )
}

// ============================================================================
// Result types
// ============================================================================

#[derive(Debug, Clone)]
pub struct EnhanceResult {
    pub title: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GenerateResult {
    pub command: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Enhance,
    Describe,
    Generate,
    Explain,
}

#[derive(Debug)]
pub enum JobOutput {
    Enhance(EnhanceResult),
    Text(String),
    Generate(GenerateResult),
}

#[derive(Debug)]
pub struct JobResult {
    pub kind: JobKind,
    pub target_id: Option<i64>,
    pub cmd_for_cache: Option<String>,
    pub outcome: Result<JobOutput>,
}

struct PendingJob {
    kind: JobKind,
    target_id: Option<i64>,
    cmd_for_cache: Option<String>,
    receiver: mpsc::Receiver<Result<JobOutput>>,
    #[allow(dead_code)]
    started_at: Instant,
}

// ============================================================================
// Manager
// ============================================================================

pub struct LlmManager {
    pub config: LlmConfig,
    pending: Vec<PendingJob>,
    cache: HashMap<String, EnhanceResult>,
    recent_calls: VecDeque<Instant>,
    blocklist: Vec<String>,
    // Cached API key + source so UI threads never block on Keychain.
    cached_api_key: Option<String>,
    cached_api_key_source: ApiKeySource,
}

impl LlmManager {
    pub fn load() -> Self {
        let config = LlmConfig::load_or_default();
        let mut s = Self {
            config,
            pending: Vec::new(),
            cache: HashMap::new(),
            recent_calls: VecDeque::new(),
            blocklist: default_blocklist(),
            cached_api_key: None,
            cached_api_key_source: ApiKeySource::None,
        };
        s.refresh_api_key_cache();
        s
    }

    pub fn save_config(&mut self) -> Result<()> {
        self.config.save()?;
        // Provider may have changed → re-resolve key.
        self.refresh_api_key_cache();
        Ok(())
    }

    /// Resolve api key (env first, keychain second) and cache result.
    /// This is the ONLY place keyring is touched on the UI thread — done
    /// once at startup + after config save / explicit refresh.
    pub fn refresh_api_key_cache(&mut self) {
        if let Ok(v) = std::env::var(&self.config.api_key_env) {
            if !v.trim().is_empty() {
                self.cached_api_key = Some(v);
                self.cached_api_key_source = ApiKeySource::Env;
                return;
            }
        }
        if let Some(v) = keyring_get(&self.config.api_key_env) {
            self.cached_api_key = Some(v);
            self.cached_api_key_source = ApiKeySource::Keychain;
            return;
        }
        self.cached_api_key = None;
        self.cached_api_key_source = ApiKeySource::None;
    }

    /// Cache-only is_configured (no keyring access).
    pub fn is_configured(&self) -> bool {
        if self.config.model.trim().is_empty() {
            return false;
        }
        self.config.is_local() || self.cached_api_key.is_some()
    }

    pub fn api_key_source(&self) -> ApiKeySource {
        self.cached_api_key_source
    }

    pub fn missing_reason(&self) -> Option<String> {
        if self.config.model.trim().is_empty() {
            return Some("未配置模型名".into());
        }
        if !self.config.is_local() && self.cached_api_key.is_none() {
            return Some(format!(
                "API key 未配置（环境变量 {} 或在设置中保存到系统 Keychain）",
                self.config.api_key_env
            ));
        }
        None
    }

    pub fn in_flight(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn paste_enhance_active(&self) -> bool {
        self.pending.iter().any(|j| j.kind == JobKind::Enhance)
    }

    pub fn is_busy(&self, kind: JobKind) -> bool {
        self.pending.iter().any(|j| j.kind == kind)
    }

    /// Returns true if the command is in the cache.
    #[allow(dead_code)]
    pub fn cached_enhance(&self, command: &str) -> Option<&EnhanceResult> {
        self.cache.get(&hash(command))
    }

    fn check_rate(&mut self) -> bool {
        let now = Instant::now();
        let cutoff = now - Duration::from_secs(60);
        while let Some(&t) = self.recent_calls.front() {
            if t < cutoff {
                self.recent_calls.pop_front();
            } else {
                break;
            }
        }
        self.recent_calls.len() < self.config.rate_per_minute as usize
    }

    fn record_call(&mut self) {
        self.recent_calls.push_back(Instant::now());
    }

    fn is_dangerous(&self, command: &str) -> bool {
        let c = command.trim();
        self.blocklist.iter().any(|pat| c.contains(pat.as_str()))
    }

    pub fn can_paste_enhance(&self, command: &str) -> bool {
        self.config.features.paste_enhance
            && self.config.consent.paste_enhance.is_some()
            && self.is_configured()
            && !self.config.offline_mode
            && command.trim().len() >= 10
            && !self.is_dangerous(command)
    }

    /// Spawn an enhance job if conditions are met. Returns true if spawned (or served from cache).
    pub fn maybe_enhance(
        &mut self,
        command: &str,
        rule_title: &str,
        rule_category: &str,
        rule_tags: &[String],
    ) -> EnhanceTrigger {
        if !self.can_paste_enhance(command) {
            return EnhanceTrigger::Skipped;
        }
        if let Some(cached) = self.cache.get(&hash(command)).cloned() {
            return EnhanceTrigger::Cached(cached);
        }
        if self.paste_enhance_active() {
            return EnhanceTrigger::Skipped;
        }
        if !self.check_rate() {
            return EnhanceTrigger::Skipped;
        }
        self.record_call();

        let config = self.config.clone();
        let api_key = self.cached_api_key.clone().unwrap_or_default();
        let cmd = command.to_string();
        let rule_title = rule_title.to_string();
        let rule_category = rule_category.to_string();
        let rule_tags_str = rule_tags.join(", ");

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let user = prompt_enhance(&cmd, &rule_title, &rule_category, &rule_tags_str);
            let result = complete(&config, &api_key, SYS_ENHANCE, &user)
                .and_then(|text| parse_enhance(&text))
                .map(JobOutput::Enhance);
            let _ = tx.send(result);
        });

        self.pending.push(PendingJob {
            kind: JobKind::Enhance,
            target_id: None,
            cmd_for_cache: Some(command.to_string()),
            receiver: rx,
            started_at: Instant::now(),
        });
        EnhanceTrigger::Spawned
    }

    pub fn generate_description(&mut self, title: &str, command: &str, target_id: Option<i64>) {
        if !self.is_configured() || self.config.offline_mode {
            return;
        }
        let config = self.config.clone();
        let api_key = self.cached_api_key.clone().unwrap_or_default();
        let user = prompt_describe(title, command);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = complete(&config, &api_key, SYS_DESCRIBE, &user).map(JobOutput::Text);
            let _ = tx.send(result);
        });
        self.record_call();
        self.pending.push(PendingJob {
            kind: JobKind::Describe,
            target_id,
            cmd_for_cache: None,
            receiver: rx,
            started_at: Instant::now(),
        });
    }

    pub fn generate_command(&mut self, prompt: &str) {
        if !self.is_configured() || self.config.offline_mode {
            return;
        }
        let config = self.config.clone();
        let api_key = self.cached_api_key.clone().unwrap_or_default();
        let user = prompt_generate(prompt);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = complete(&config, &api_key, SYS_GENERATE, &user)
                .and_then(|text| parse_generate(&text))
                .map(JobOutput::Generate);
            let _ = tx.send(result);
        });
        self.record_call();
        self.pending.push(PendingJob {
            kind: JobKind::Generate,
            target_id: None,
            cmd_for_cache: None,
            receiver: rx,
            started_at: Instant::now(),
        });
    }

    pub fn explain(&mut self, command: &str, target_id: i64) {
        if !self.is_configured() || self.config.offline_mode {
            return;
        }
        let config = self.config.clone();
        let api_key = self.cached_api_key.clone().unwrap_or_default();
        let user = prompt_explain(command);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = complete(&config, &api_key, SYS_EXPLAIN, &user).map(JobOutput::Text);
            let _ = tx.send(result);
        });
        self.record_call();
        self.pending.push(PendingJob {
            kind: JobKind::Explain,
            target_id: Some(target_id),
            cmd_for_cache: None,
            receiver: rx,
            started_at: Instant::now(),
        });
    }

    /// Drain finished jobs. Called every UI frame.
    pub fn poll(&mut self) -> Vec<JobResult> {
        let mut done = Vec::new();
        let mut still_pending = Vec::new();
        let mut to_cache: Vec<(String, EnhanceResult)> = Vec::new();
        for job in self.pending.drain(..) {
            match job.receiver.try_recv() {
                Ok(outcome) => {
                    if job.kind == JobKind::Enhance {
                        if let (Ok(JobOutput::Enhance(r)), Some(cmd)) =
                            (&outcome, &job.cmd_for_cache)
                        {
                            to_cache.push((cmd.clone(), r.clone()));
                        }
                    }
                    done.push(JobResult {
                        kind: job.kind,
                        target_id: job.target_id,
                        cmd_for_cache: job.cmd_for_cache.clone(),
                        outcome,
                    });
                }
                Err(mpsc::TryRecvError::Empty) => still_pending.push(job),
                Err(mpsc::TryRecvError::Disconnected) => done.push(JobResult {
                    kind: job.kind,
                    target_id: job.target_id,
                    cmd_for_cache: job.cmd_for_cache,
                    outcome: Err(anyhow!("worker died")),
                }),
            }
        }
        self.pending = still_pending;
        for (cmd, r) in to_cache {
            self.cache.insert(hash(&cmd), r);
        }
        done
    }

    #[allow(dead_code)]
    pub fn remember_enhance(&mut self, command: &str, result: EnhanceResult) {
        self.cache.insert(hash(command), result);
    }

    pub fn test_connection(&self) -> Result<String> {
        let api_key = self.cached_api_key.clone().unwrap_or_default();
        complete(
            &self.config,
            &api_key,
            "你是测试助手，回复 'ok'。",
            "请回复 'ok'。",
        )
    }
}

pub enum EnhanceTrigger {
    Spawned,
    Cached(EnhanceResult),
    Skipped,
}

fn hash(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

fn default_blocklist() -> Vec<String> {
    vec![
        "rm -rf /".into(),
        "rm -rf /*".into(),
        ":(){:|:&};:".into(),
        "mkfs.".into(),
        "dd if=/dev/zero of=/dev/".into(),
        "> /dev/sda".into(),
    ]
}

// ============================================================================
// Response parsing
// ============================================================================

fn extract_json(s: &str) -> &str {
    let t = s.trim();
    // strip ```json ... ``` if present
    if let Some(stripped) = t.strip_prefix("```json") {
        if let Some(end) = stripped.rfind("```") {
            return stripped[..end].trim();
        }
    }
    if let Some(stripped) = t.strip_prefix("```") {
        if let Some(end) = stripped.rfind("```") {
            return stripped[..end].trim();
        }
    }
    t
}

fn parse_enhance(s: &str) -> Result<EnhanceResult> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
    }
    let raw: Raw = serde_json::from_str(extract_json(s)).context("parse enhance json")?;
    let tags = raw
        .tags
        .into_iter()
        .map(|t| t.trim().trim_start_matches('#').to_lowercase())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    Ok(EnhanceResult {
        title: raw.title.filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()),
        description: raw
            .description
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string()),
        tags,
    })
}

fn parse_generate(s: &str) -> Result<GenerateResult> {
    #[derive(Deserialize)]
    struct Raw {
        command: String,
        #[serde(default)]
        description: String,
    }
    let raw: Raw = serde_json::from_str(extract_json(s)).context("parse generate json")?;
    Ok(GenerateResult {
        command: raw.command.trim().to_string(),
        description: raw.description.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_enhance_json() {
        let s = "{\"title\":\"测试\",\"description\":\"## hi\",\"tags\":[\"a\",\"b\"]}";
        let r = parse_enhance(s).unwrap();
        assert_eq!(r.title.as_deref(), Some("测试"));
        assert_eq!(r.tags, vec!["a", "b"]);
    }

    #[test]
    fn parses_fenced_enhance_json() {
        let s = "```json\n{\"title\":\"t\",\"description\":\"d\",\"tags\":[]}\n```";
        let r = parse_enhance(s).unwrap();
        assert_eq!(r.title.as_deref(), Some("t"));
    }

    #[test]
    fn parses_generate_json() {
        let s = r#"{"command":"ls -la","description":"List files"}"#;
        let r = parse_generate(s).unwrap();
        assert_eq!(r.command, "ls -la");
    }

    #[test]
    fn blocklist_catches_dangerous() {
        let m = LlmManager::load();
        assert!(m.is_dangerous("rm -rf /"));
        assert!(m.is_dangerous("sudo rm -rf /etc"));
    }

    #[test]
    fn rate_limiter_works() {
        let mut m = LlmManager::load();
        m.config.rate_per_minute = 3;
        assert!(m.check_rate());
        m.record_call();
        assert!(m.check_rate());
        m.record_call();
        assert!(m.check_rate());
        m.record_call();
        assert!(!m.check_rate());
    }

    #[test]
    fn extract_json_strips_code_fence() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(extract_json("```\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(extract_json("  {\"a\":1}  "), "{\"a\":1}");
    }

    #[test]
    fn parse_enhance_tolerates_missing_fields() {
        let r = parse_enhance("{\"tags\":[]}").unwrap();
        assert_eq!(r.title, None);
        assert_eq!(r.description, None);
        assert!(r.tags.is_empty());
    }

    #[test]
    fn parse_enhance_lowercases_and_strips_hash() {
        let r = parse_enhance("{\"tags\":[\"#Docker\", \" Cleanup \", \"\"]}").unwrap();
        assert_eq!(r.tags, vec!["docker", "cleanup"]);
    }

    #[test]
    fn parse_enhance_rejects_garbage() {
        assert!(parse_enhance("not json").is_err());
    }

    #[test]
    fn config_default_is_openai() {
        let c = LlmConfig::default();
        assert_eq!(c.provider, Provider::OpenAi);
        assert!(c.base_url.contains("openai.com"));
        assert_eq!(c.api_key_env, "OPENAI_API_KEY");
    }

    #[test]
    fn config_serializes_round_trip() {
        let mut c = LlmConfig::default();
        c.provider = Provider::Anthropic;
        c.model = "claude-sonnet-4-6".into();
        c.features.paste_enhance = true;
        c.consent.paste_enhance = Some(Utc::now());
        let s = toml::to_string_pretty(&c).unwrap();
        let back: LlmConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.provider, Provider::Anthropic);
        assert_eq!(back.model, "claude-sonnet-4-6");
        assert!(back.features.paste_enhance);
        assert!(back.consent.paste_enhance.is_some());
    }

    #[test]
    fn is_local_detects_localhost_and_127() {
        let mut c = LlmConfig::default();
        c.base_url = "http://localhost:11434/v1".into();
        assert!(c.is_local());
        c.base_url = "http://127.0.0.1:11434/v1".into();
        assert!(c.is_local());
        c.base_url = "https://api.openai.com/v1".into();
        assert!(!c.is_local());
    }

    #[test]
    fn missing_reason_flags_empty_model() {
        let mut m = LlmManager::load();
        m.config.model = "".into();
        assert!(m.missing_reason().is_some());
    }

    #[test]
    fn cant_paste_enhance_without_consent() {
        let mut m = LlmManager::load();
        m.config.features.paste_enhance = true;
        m.config.consent.paste_enhance = None;
        // even with consent missing, should not allow
        assert!(!m.can_paste_enhance("docker container prune -f"));
    }

    #[test]
    fn cant_paste_enhance_for_short_command() {
        let mut m = LlmManager::load();
        m.config.features.paste_enhance = true;
        m.config.consent.paste_enhance = Some(Utc::now());
        m.config.base_url = "http://localhost:11434/v1".into(); // is_configured via local
        m.config.model = "llama".into();
        assert!(!m.can_paste_enhance("ls"), "command too short");
        assert!(m.can_paste_enhance("ls -la /var"), "long enough");
    }

    #[test]
    fn cant_paste_enhance_for_dangerous_command() {
        let mut m = LlmManager::load();
        m.config.features.paste_enhance = true;
        m.config.consent.paste_enhance = Some(Utc::now());
        m.config.base_url = "http://localhost:11434/v1".into();
        m.config.model = "llama".into();
        assert!(!m.can_paste_enhance("sudo rm -rf / --no-preserve-root"));
    }

    #[test]
    fn cant_paste_enhance_in_offline_mode() {
        let mut m = LlmManager::load();
        m.config.features.paste_enhance = true;
        m.config.consent.paste_enhance = Some(Utc::now());
        m.config.base_url = "http://localhost:11434/v1".into();
        m.config.model = "llama".into();
        m.config.offline_mode = true;
        assert!(!m.can_paste_enhance("docker ps -a"));
    }
}
