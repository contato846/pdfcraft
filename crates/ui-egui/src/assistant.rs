//! Assistant: a conversation about the open document with the user's AI (Claude, ChatGPT,
//! Gemini, or a local model through Ollama), in a side panel.
//!
//! Each question goes out with the text of the current page or of the whole document, page by
//! page, and the conversation so far; the answer streams in as it is written. The text comes from
//! the document as it is now (unsaved changes included), read on a worker thread. Scanned pages
//! have no text until Recognize text has run on them.
//!
//! This crate has no network code: the desktop app supplies [`AiTransport`], which sends a call
//! and hands back the answer line by line (server-sent events and NDJSON are both line based).
//! The key is the user's own and goes only to the provider they chose. It stays in memory unless
//! they ask for it to be remembered on this computer.

use std::sync::{Arc, Mutex};

use pdfcraft_engine::DocId;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{PdfCraftApp, widgets};

/// One HTTP call.
#[derive(Clone, Debug)]
pub struct HttpCall {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// Sends `call` and hands each line of the answer to `line` with the HTTP status (`line` returns
/// `false` to stop reading); returns the status. Error statuses are not raised: their body comes
/// line by line too, so the provider's own message can be shown.
pub type AiTransport = Arc<dyn Fn(&HttpCall, &mut dyn FnMut(u16, &str) -> bool) -> Result<u16, String> + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Provider {
    #[default]
    Claude,
    OpenAi,
    Gemini,
    Ollama,
}

/// The default Claude model.
pub const CLAUDE_MODEL: &str = "claude-opus-5-5";

/// Claude models that take the server-side fallback on a refusal (`fallbacks: "default"`).
const CLAUDE_FALLBACK_MODELS: [&str; 4] = ["claude-opus-5-5", "claude-fable-5-1", "claude-opus-5", "claude-sonnet-5-5"];

impl Provider {
    pub const ALL: [Provider; 4] = [Provider::Claude, Provider::OpenAi, Provider::Gemini, Provider::Ollama];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude (Anthropic)",
            Provider::OpenAi => "ChatGPT (OpenAI)",
            Provider::Gemini => "Gemini (Google)",
            Provider::Ollama => "Local model (Ollama)",
        }
    }

    fn index(self) -> usize {
        match self {
            Provider::Claude => 0,
            Provider::OpenAi => 1,
            Provider::Gemini => 2,
            Provider::Ollama => 3,
        }
    }

    pub fn needs_key(self) -> bool {
        self != Provider::Ollama
    }

    /// Where the user gets a key.
    pub fn key_page(self) -> &'static str {
        match self {
            Provider::Claude => "https://console.anthropic.com/settings/keys",
            Provider::OpenAi => "https://platform.openai.com/api-keys",
            Provider::Gemini => "https://aistudio.google.com/apikey",
            Provider::Ollama => "https://ollama.com/download",
        }
    }

    /// The most document text sent, in characters (a little under each family's context window;
    /// a local model gets far less, as its window is usually small).
    fn max_chars(self) -> usize {
        match self {
            Provider::Claude | Provider::Gemini => 2_000_000,
            Provider::OpenAi => 400_000,
            Provider::Ollama => 60_000,
        }
    }
}

/// What the question is about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Page,
    Document,
}

/// One message of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub user: bool,
    pub text: String,
}

/// Settings kept between sessions (the keys only when the user asks).
#[derive(Clone, Debug, PartialEq)]
pub struct AssistantSettings {
    pub provider: Provider,
    pub models: [String; 4],
    pub keys: [String; 4],
    pub remember_keys: bool,
    pub ollama_url: String,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            provider: Provider::Claude,
            models: [CLAUDE_MODEL.into(), String::new(), String::new(), String::new()],
            keys: Default::default(),
            remember_keys: false,
            ollama_url: "http://localhost:11434".into(),
        }
    }
}

impl AssistantSettings {
    pub fn to_json(&self) -> Value {
        json!({
            "provider": self.provider,
            "models": self.models,
            "remember_keys": self.remember_keys,
            "keys": if self.remember_keys { json!(self.keys) } else { Value::Null },
            "ollama_url": self.ollama_url,
        })
    }

    pub fn from_json(v: &Value) -> Self {
        let mut s = Self::default();
        if let Ok(p) = serde_json::from_value::<Provider>(v["provider"].clone()) {
            s.provider = p;
        }
        let strings = |v: &Value| -> Option<[String; 4]> {
            let a = v.as_array()?;
            let mut out: [String; 4] = Default::default();
            for (o, x) in out.iter_mut().zip(a) {
                *o = x.as_str().unwrap_or_default().chars().take(400).collect();
            }
            Some(out)
        };
        if let Some(m) = strings(&v["models"]) {
            s.models = m;
        }
        if s.models[0].trim().is_empty() {
            s.models[0] = CLAUDE_MODEL.into();
        }
        s.remember_keys = v["remember_keys"].as_bool().unwrap_or(false);
        if s.remember_keys
            && let Some(k) = strings(&v["keys"])
        {
            s.keys = k;
        }
        if let Some(u) = v["ollama_url"].as_str().filter(|u| u.starts_with("http://") || u.starts_with("https://")) {
            s.ollama_url = u.chars().take(400).collect();
        }
        s
    }
}

/// A question being answered, shared with its worker thread.
#[derive(Default)]
pub struct Progress {
    pub reply: String,
    pub done: bool,
    pub error: Option<String>,
    pub note: Option<String>,
    pub cancel: bool,
}

/// The model list being fetched from a provider.
#[derive(Default)]
struct ModelFetch {
    provider: Option<Provider>,
    result: Option<Result<Vec<String>, String>>,
}

/// The Assistant panel.
#[derive(Default)]
pub struct Assistant {
    pub settings: AssistantSettings,
    pub turns: Vec<Turn>,
    pub input: String,
    pub scope: Scope,
    /// The document the conversation is about (another document starts a new one).
    pub doc: Option<DocId>,
    run: Option<Arc<Mutex<Progress>>>,
    models: [Vec<String>; 4],
    fetch: Option<Arc<Mutex<ModelFetch>>>,
    pub status: Option<String>,
    show_settings: bool,
}

impl Assistant {
    pub fn busy(&self) -> bool {
        self.run.is_some()
    }
}

const SYSTEM: &str = "You are the assistant inside PdfCraft, a PDF editor. The user has a document open; its text is \
below, page by page, each page introduced by a line such as `=== Page 12 ===` (the page number as the viewer shows it). \
Answer from the document. When you rely on a passage, cite its page as (p. 12). If the document doesn't say, say so \
plainly instead of guessing. Answer in the language the user writes in, and keep the formatting simple.";

/// The document text for the request: `pages` (0-based) with their markers, at most `max` characters.
/// Returns the text and, when it had to stop early, the last page included (1-based).
pub fn document_text(pages: &[(usize, String)], max: usize) -> (String, Option<usize>) {
    let mut out = String::new();
    for (i, (page, text)) in pages.iter().enumerate() {
        let block = format!("=== Page {} ===\n{}\n\n", page + 1, text.trim());
        if out.len() + block.len() > max {
            let last = i.checked_sub(1).and_then(|j| pages.get(j)).map(|(p, _)| p + 1);
            return (out, Some(last.unwrap_or(0)));
        }
        out.push_str(&block);
    }
    (out, None)
}

/// The request for `provider`: `history` is the conversation, the last turn the question.
pub fn request(s: &AssistantSettings, provider: Provider, system: &str, history: &[Turn]) -> HttpCall {
    let model = s.models[provider.index()].trim().to_owned();
    let key = s.keys[provider.index()].trim().to_owned();
    let json_headers = |extra: Vec<(String, String)>| {
        let mut h = vec![("content-type".to_owned(), "application/json".to_owned())];
        h.extend(extra);
        h
    };
    match provider {
        Provider::Claude => {
            let messages: Vec<Value> =
                history.iter().map(|t| json!({"role": if t.user { "user" } else { "assistant" }, "content": t.text})).collect();
            let mut body = json!({
                "model": model,
                "max_tokens": 32000,
                "stream": true,
                // The document is the long, stable part: cached, so follow-up questions about it
                // are read from the cache.
                "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
                "messages": messages,
            });
            let mut headers = vec![("x-api-key".to_owned(), key), ("anthropic-version".to_owned(), "2023-06-01".to_owned())];
            // A refusal by the safety classifiers is retried on a fallback model chosen by the
            // server, where the model takes it.
            if CLAUDE_FALLBACK_MODELS.contains(&model.as_str()) {
                body["fallbacks"] = json!("default");
                headers.push(("anthropic-beta".to_owned(), "server-side-fallback-2026-07-01".to_owned()));
            }
            HttpCall {
                method: "POST",
                url: "https://api.anthropic.com/v1/messages".into(),
                headers: json_headers(headers),
                body: Some(body.to_string()),
            }
        }
        Provider::OpenAi => {
            let mut messages = vec![json!({"role": "system", "content": system})];
            messages.extend(history.iter().map(|t| json!({"role": if t.user { "user" } else { "assistant" }, "content": t.text})));
            let body = json!({"model": model, "stream": true, "messages": messages});
            HttpCall {
                method: "POST",
                url: "https://api.openai.com/v1/chat/completions".into(),
                headers: json_headers(vec![("authorization".to_owned(), format!("Bearer {key}"))]),
                body: Some(body.to_string()),
            }
        }
        Provider::Gemini => {
            let contents: Vec<Value> =
                history.iter().map(|t| json!({"role": if t.user { "user" } else { "model" }, "parts": [{"text": t.text}]})).collect();
            let body = json!({"system_instruction": {"parts": [{"text": system}]}, "contents": contents});
            HttpCall {
                method: "POST",
                url: format!("https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse", url_segment(&model)),
                headers: json_headers(vec![("x-goog-api-key".to_owned(), key)]),
                body: Some(body.to_string()),
            }
        }
        Provider::Ollama => {
            let mut messages = vec![json!({"role": "system", "content": system})];
            messages.extend(history.iter().map(|t| json!({"role": if t.user { "user" } else { "assistant" }, "content": t.text})));
            let body = json!({"model": model, "stream": true, "messages": messages});
            HttpCall {
                method: "POST",
                url: format!("{}/api/chat", s.ollama_url.trim_end_matches('/')),
                headers: json_headers(Vec::new()),
                body: Some(body.to_string()),
            }
        }
    }
}

/// A model name as one URL path segment (no slashes, queries or fragments slip in).
fn url_segment(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect()
}

/// What one line of a streamed answer adds.
#[derive(Debug, Default, PartialEq)]
pub struct Piece {
    pub text: String,
    pub error: Option<String>,
    pub refused: bool,
    pub done: bool,
}

/// Read one line of `provider`'s streamed answer.
pub fn parse_line(provider: Provider, line: &str) -> Piece {
    let mut p = Piece::default();
    let data = match provider {
        Provider::Ollama => line.trim(),
        _ => match line.strip_prefix("data:") {
            Some(d) => d.trim(),
            None => return p,
        },
    };
    if data.is_empty() {
        return p;
    }
    if data == "[DONE]" {
        p.done = true;
        return p;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else { return p };
    match provider {
        Provider::Claude => match v["type"].as_str() {
            Some("content_block_delta") if v["delta"]["type"] == "text_delta" => p.text = v["delta"]["text"].as_str().unwrap_or_default().into(),
            Some("message_delta") => p.refused = v["delta"]["stop_reason"] == "refusal",
            Some("message_stop") => p.done = true,
            Some("error") => p.error = Some(error_message(&v)),
            _ => {}
        },
        Provider::OpenAi => {
            if v.get("error").is_some() {
                p.error = Some(error_message(&v));
            } else if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                p.text = t.into();
            }
        }
        Provider::Gemini => {
            if v.get("error").is_some() {
                p.error = Some(error_message(&v));
            } else if let Some(parts) = v["candidates"][0]["content"]["parts"].as_array() {
                p.text = parts.iter().filter_map(|x| x["text"].as_str()).collect();
            }
        }
        Provider::Ollama => {
            if v.get("error").is_some() {
                p.error = Some(error_message(&v));
            } else {
                p.text = v["message"]["content"].as_str().unwrap_or_default().into();
                p.done = v["done"].as_bool().unwrap_or(false);
            }
        }
    }
    p
}

/// The message in a provider's error answer (each nests it differently).
pub fn error_message(v: &Value) -> String {
    let v = v.as_array().and_then(|a| a.first()).unwrap_or(v);
    let e = &v["error"];
    e["message"]
        .as_str()
        .or_else(|| e.as_str())
        .or_else(|| v["message"].as_str())
        .map_or_else(|| v.to_string(), str::to_owned)
        .chars()
        .take(500)
        .collect()
}

/// The call that lists `provider`'s models.
pub fn models_request(s: &AssistantSettings, provider: Provider) -> HttpCall {
    let key = s.keys[provider.index()].trim().to_owned();
    let (url, headers) = match provider {
        Provider::Claude => (
            "https://api.anthropic.com/v1/models?limit=100".to_owned(),
            vec![("x-api-key".to_owned(), key), ("anthropic-version".to_owned(), "2023-06-01".to_owned())],
        ),
        Provider::OpenAi => ("https://api.openai.com/v1/models".to_owned(), vec![("authorization".to_owned(), format!("Bearer {key}"))]),
        Provider::Gemini => {
            ("https://generativelanguage.googleapis.com/v1beta/models?pageSize=200".to_owned(), vec![("x-goog-api-key".to_owned(), key)])
        }
        Provider::Ollama => (format!("{}/api/tags", s.ollama_url.trim_end_matches('/')), Vec::new()),
    };
    HttpCall { method: "GET", url, headers, body: None }
}

/// The chat models in a model list answer.
pub fn parse_models(provider: Provider, body: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "unexpected answer".to_owned())?;
    if v.get("error").is_some() {
        return Err(error_message(&v));
    }
    let mut out: Vec<String> = match provider {
        Provider::Claude => v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect()).unwrap_or_default(),
        Provider::OpenAi => {
            const NOT_CHAT: [&str; 9] = ["audio", "realtime", "tts", "transcribe", "image", "embedding", "moderation", "search", "dall-e"];
            v["data"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m["id"].as_str())
                        .filter(|id| {
                            let chat = id.starts_with("gpt")
                                || id.starts_with("chatgpt")
                                || (id.starts_with('o') && id.chars().nth(1).is_some_and(|c| c.is_ascii_digit()));
                            chat && !NOT_CHAT.iter().any(|n| id.contains(n))
                        })
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        }
        Provider::Gemini => v["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|m| m["supportedGenerationMethods"].as_array().is_some_and(|g| g.iter().any(|x| x == "generateContent")))
                    .filter_map(|m| m["name"].as_str().map(|n| n.trim_start_matches("models/").to_owned()))
                    .collect()
            })
            .unwrap_or_default(),
        Provider::Ollama => {
            v["models"].as_array().map(|a| a.iter().filter_map(|m| m["name"].as_str().map(str::to_owned)).collect()).unwrap_or_default()
        }
    };
    if provider != Provider::Claude {
        out.sort_unstable_by(|a, b| b.cmp(a));
    }
    out.dedup();
    Ok(out)
}

impl PdfCraftApp {
    /// View ▸ Assistant: open or close the panel.
    pub fn toggle_assistant(&mut self) {
        let open = self.right == Some(crate::RightPanel::Assistant);
        self.choose_right_panel(if open { None } else { Some(crate::RightPanel::Assistant) });
    }

    /// Ask the question in the input box.
    pub fn assistant_send(&mut self) {
        let Some(transport) = self.ai_transport.clone() else {
            self.assistant.status = Some(tl!("The Assistant needs the desktop app to reach the AI.").to_owned());
            return;
        };
        let Some((index, id)) = self.active_ids() else { return };
        let a = &mut self.assistant;
        let question = a.input.trim().to_owned();
        if question.is_empty() || a.run.is_some() {
            return;
        }
        let provider = a.settings.provider;
        if provider.needs_key() && a.settings.keys[provider.index()].trim().is_empty() {
            a.show_settings = true;
            a.status = Some(tl!("Enter your API key for this AI first.").to_owned());
            return;
        }
        if a.settings.models[provider.index()].trim().is_empty() {
            a.show_settings = true;
            a.status = Some(tl!("Choose a model first (Refresh the list).").to_owned());
            return;
        }
        let Some(doc) = self.session.get(id) else { return };
        let source = doc.export_source();
        let title = doc.info.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| doc.name.clone());
        let n = doc.info.pages.len();
        let current = self.views.get(index).map_or(0, |v| v.current);
        let a = &mut self.assistant;
        if a.doc != Some(id) {
            a.turns.clear();
            a.doc = Some(id);
        }
        let pages: Vec<usize> = match a.scope {
            Scope::Page => vec![current.min(n.saturating_sub(1))],
            Scope::Document => (0..n).collect(),
        };
        a.turns.push(Turn { user: true, text: question });
        let history = a.turns.clone();
        a.turns.push(Turn { user: false, text: String::new() });
        a.input.clear();
        a.status = None;
        let settings = a.settings.clone();
        let progress = Arc::new(Mutex::new(Progress::default()));
        a.run = Some(progress.clone());
        let p = progress.clone();
        let ctx = self.ctx.clone();
        let work = move || {
            let mut exporter = pdfcraft_engine::export::Exporter::from_source(source);
            let texts: Vec<(usize, String)> = pages.iter().map(|&pg| (pg, exporter.text(pg).unwrap_or_default())).collect();
            let (text, cut) = document_text(&texts, provider.max_chars());
            if let Some(last) = cut
                && let Ok(mut s) = p.lock()
            {
                s.note =
                    Some(crate::i18n::fmt(tl!("The document is too long for this AI: it got the text up to page {n}."), &[("n", &last.to_string())]));
            }
            if text.trim().is_empty()
                && let Ok(mut s) = p.lock()
            {
                s.note = Some(tl!("These pages have no text (scanned pages need Recognize text first).").to_owned());
            }
            let system = format!("{SYSTEM}\n\n<document title=\"{}\">\n{text}</document>", title.replace('"', "'"));
            let call = request(&settings, provider, &system, &history);
            let mut failed_body = String::new();
            let result = transport(&call, &mut |status: u16, line: &str| {
                let Ok(mut s) = p.lock() else { return false };
                if s.cancel {
                    return false;
                }
                // An error answer is one JSON document, read whole and shown at the end.
                if !(200..300).contains(&status) {
                    if failed_body.len() < 8000 {
                        failed_body.push_str(line);
                        failed_body.push('\n');
                    }
                    return true;
                }
                let piece = parse_line(provider, line);
                s.reply.push_str(&piece.text);
                if piece.refused {
                    s.note = Some(tl!("The AI declined to answer this.").to_owned());
                }
                if let Some(e) = piece.error {
                    s.error = Some(e);
                    return false;
                }
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
                !piece.done
            });
            if let Ok(mut s) = p.lock() {
                match result {
                    Ok(code) if !(200..300).contains(&code) => {
                        let msg = serde_json::from_str::<Value>(&failed_body).map(|v| error_message(&v)).unwrap_or(failed_body.clone());
                        s.error = Some(format!("HTTP {code}: {}", msg.chars().take(500).collect::<String>()));
                        s.reply.clear();
                    }
                    Ok(_) => {}
                    Err(e) => s.error = Some(e),
                }
                s.done = true;
            }
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new().name("pdfcraft-assistant".into()).spawn(work).ok();
        #[cfg(target_arch = "wasm32")]
        work();
    }

    /// Each frame: copy the answer so far into the conversation.
    pub(crate) fn poll_assistant(&mut self) {
        let a = &mut self.assistant;
        if let Some(run) = a.run.clone() {
            let Ok(s) = run.lock() else {
                a.run = None;
                return;
            };
            if let Some(last) = a.turns.last_mut().filter(|t| !t.user) {
                last.text.clone_from(&s.reply);
            }
            if s.done {
                if let Some(e) = &s.error {
                    a.status = Some(e.clone());
                    // The question stays, so it can be asked again; the empty answer goes.
                    if a.turns.last().is_some_and(|t| !t.user && t.text.is_empty()) {
                        a.turns.pop();
                    }
                } else if let Some(n) = &s.note {
                    a.status = Some(n.clone());
                }
                drop(s);
                a.run = None;
            } else if let Some(n) = &s.note {
                a.status = Some(n.clone());
            }
        }
        if let Some(f) = a.fetch.clone() {
            let Ok(mut s) = f.lock() else {
                a.fetch = None;
                return;
            };
            if let (Some(p), Some(result)) = (s.provider, s.result.take()) {
                match result {
                    Ok(list) if list.is_empty() => a.status = Some(tl!("No models found for this key.").to_owned()),
                    Ok(list) => {
                        if !list.contains(&a.settings.models[p.index()]) {
                            a.settings.models[p.index()] = list.first().cloned().unwrap_or_default();
                        }
                        a.models[p.index()] = list;
                        a.status = None;
                    }
                    Err(e) => a.status = Some(e),
                }
                drop(s);
                a.fetch = None;
            }
        }
    }

    /// Stop the answer being written (what came so far stays).
    pub fn assistant_stop(&mut self) {
        if let Some(run) = &self.assistant.run
            && let Ok(mut s) = run.lock()
        {
            s.cancel = true;
            s.done = true;
        }
    }

    pub(crate) fn assistant_fetch_models(&mut self) {
        let Some(transport) = self.ai_transport.clone() else { return };
        let a = &mut self.assistant;
        let provider = a.settings.provider;
        if provider.needs_key() && a.settings.keys[provider.index()].trim().is_empty() {
            a.status = Some(tl!("Enter your API key for this AI first.").to_owned());
            return;
        }
        let call = models_request(&a.settings, provider);
        let fetch = Arc::new(Mutex::new(ModelFetch { provider: Some(provider), result: None }));
        a.fetch = Some(fetch.clone());
        let ctx = self.ctx.clone();
        let work = move || {
            let mut body = String::new();
            let result = transport(&call, &mut |_, line: &str| {
                if body.len() < 4 << 20 {
                    body.push_str(line);
                    body.push('\n');
                }
                true
            });
            let result = match result {
                Ok(code) if (200..300).contains(&code) => parse_models(provider, &body),
                Ok(code) => Err(format!(
                    "HTTP {code}: {}",
                    serde_json::from_str::<Value>(&body).map(|v| error_message(&v)).unwrap_or_else(|_| body.chars().take(300).collect())
                )),
                Err(e) => Err(e),
            };
            if let Ok(mut s) = fetch.lock() {
                s.result = Some(result);
            }
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new().name("pdfcraft-assistant-models".into()).spawn(work).ok();
        #[cfg(target_arch = "wasm32")]
        work();
    }
}

/// What the panel asks the app to do once it is drawn.
pub enum PanelAction {
    Send,
    Stop,
    FetchModels,
    GoToPage(usize),
}

/// The panel's body.
pub(crate) fn panel(ui: &mut egui::Ui, t: &Tokens, a: &mut Assistant, has_transport: bool) -> Option<PanelAction> {
    let mut action = None;
    let s = &mut a.settings;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("assistant-provider").selected_text(tl!(s.provider.label())).width(ui.available_width() - 34.0).show_ui(
            ui,
            |ui| {
                for p in Provider::ALL {
                    ui.selectable_value(&mut s.provider, p, tl!(p.label()));
                }
            },
        );
        if crate::icons::button(ui, "settings-2", 26.0, a.show_settings, tl!("Assistant settings")).clicked() {
            a.show_settings = !a.show_settings;
        }
    });
    let p = s.provider;
    let missing_key = p.needs_key() && s.keys[p.index()].trim().is_empty();
    if a.show_settings || missing_key {
        egui::Frame::new().fill(t.hover).corner_radius(egui::CornerRadius::same(6)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if p.needs_key() {
                ui.label(egui::RichText::new(tl!("API key")).small().color(t.text_muted));
                ui.add(
                    widgets::line(&mut s.keys[p.index()])
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .background_color(t.field)
                        .id_salt(("ai-key", p.index())),
                );
                ui.hyperlink_to(egui::RichText::new(tl!("Get a key")).small(), p.key_page());
                ui.add(widgets::checkbox(&mut s.remember_keys, tl!("Remember the keys on this computer")));
            } else {
                ui.label(egui::RichText::new(tl!("Ollama address")).small().color(t.text_muted));
                ui.add(widgets::line(&mut s.ollama_url).desired_width(f32::INFINITY).background_color(t.field).id_salt("ai-ollama"));
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tl!("Model")).small().color(t.text_muted));
            ui.horizontal(|ui| {
                let list = &a.models[p.index()];
                let model = &mut s.models[p.index()];
                if list.is_empty() {
                    ui.add(
                        widgets::line(model).desired_width(ui.available_width() - 34.0).background_color(t.field).id_salt(("ai-model", p.index())),
                    );
                } else {
                    egui::ComboBox::from_id_salt(("ai-models", p.index())).selected_text(model.as_str()).width(ui.available_width() - 34.0).show_ui(
                        ui,
                        |ui| {
                            for m in list {
                                ui.selectable_value(model, m.clone(), m);
                            }
                        },
                    );
                }
                let fetching = a.fetch.is_some();
                if ui
                    .add_enabled_ui(!fetching && has_transport, |ui| {
                        crate::icons::button(ui, "rotate-cw", 26.0, false, tl!("Refresh the model list"))
                    })
                    .inner
                    .clicked()
                {
                    action = Some(PanelAction::FetchModels);
                }
            });
        });
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        widgets::radio_value(ui, &mut a.scope, Scope::Page, tl!("This page"));
        widgets::radio_value(ui, &mut a.scope, Scope::Document, tl!("Whole document"));
    });
    ui.add_space(4.0);
    let input_height = 92.0;
    egui::ScrollArea::vertical()
        .max_height((ui.available_height() - input_height - 40.0).max(80.0))
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if a.turns.is_empty() {
                ui.label(
                    egui::RichText::new(tl!("Ask about the open document: a summary, where something is said, what a passage means…"))
                        .color(t.text_faint),
                );
            }
            for turn in &a.turns {
                let (fill, align) = if turn.user { (t.accent.gamma_multiply(0.14), egui::Align::Max) } else { (t.hover, egui::Align::Min) };
                ui.with_layout(egui::Layout::top_down(align), |ui| {
                    egui::Frame::new().fill(fill).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(10, 7)).show(
                        ui,
                        |ui| {
                            ui.set_max_width(ui.available_width() * 0.92);
                            if turn.text.is_empty() && !turn.user {
                                ui.add(egui::Spinner::new().size(14.0));
                            } else {
                                if let Some(page) = cited_pages_ui(ui, &turn.text, t) {
                                    action = Some(PanelAction::GoToPage(page));
                                }
                            }
                        },
                    );
                });
                ui.add_space(6.0);
            }
        });
    if let Some(s) = &a.status {
        ui.label(egui::RichText::new(s).small().color(t.text_muted));
    }
    let busy = a.run.is_some();
    let label = ui.label(egui::RichText::new(tl!("Your question")).small().color(t.text_muted));
    let edit = ui
        .add(
            egui::TextEdit::multiline(&mut a.input)
                .hint_text(tl!("Ask a question (Enter sends, Shift+Enter for a new line)"))
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .background_color(t.field)
                .id_salt("assistant-input"),
        )
        .labelled_by(label.id);
    let enter = edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    if enter {
        // The newline Enter just typed is not part of the question.
        while a.input.ends_with('\n') {
            a.input.pop();
        }
    }
    ui.horizontal(|ui| {
        if busy {
            if widgets::pill_button(ui, tl!("Stop"), false).clicked() {
                action = Some(PanelAction::Stop);
            }
        } else if (ui.add_enabled_ui(!a.input.trim().is_empty(), |ui| widgets::pill_button(ui, tl!("Send"), true)).inner.clicked() || enter)
            && !a.input.trim().is_empty()
        {
            action = Some(PanelAction::Send);
        }
        if !a.turns.is_empty() && !busy && ui.add(egui::Button::new(egui::RichText::new(tl!("New conversation")).small()).frame(false)).clicked() {
            a.turns.clear();
            a.status = None;
        }
    });
    action
}

/// The answer's text, with every "(p. N)" it cites as a link to that page. Returns the page
/// clicked (0-based).
fn cited_pages_ui(ui: &mut egui::Ui, text: &str, t: &Tokens) -> Option<usize> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut rest = text;
        while let Some(at) = rest.find("(p. ") {
            let (before, after) = rest.split_at(at);
            if !before.is_empty() {
                ui.add(egui::Label::new(before).selectable(true));
            }
            // "(p. " and the digits are ASCII: every offset below is a character boundary.
            let digits: String = after.get("(p. ".len()..).unwrap_or_default().chars().take_while(char::is_ascii_digit).collect();
            let close = "(p. ".len() + digits.len();
            if !digits.is_empty() && after.get(close..close + 1) == Some(")") {
                let n: usize = digits.parse().unwrap_or(0);
                let cite = after.get(..close + 1).unwrap_or_default();
                if ui
                    .add(egui::Label::new(egui::RichText::new(cite).color(t.accent_text)).sense(egui::Sense::click()))
                    .on_hover_text(tl!("Go to the page"))
                    .clicked()
                    && n > 0
                {
                    clicked = Some(n - 1);
                }
                rest = after.get(close + 1..).unwrap_or_default();
            } else {
                ui.add(egui::Label::new("(").selectable(true));
                rest = after.get(1..).unwrap_or_default();
            }
        }
        if !rest.is_empty() {
            ui.add(egui::Label::new(rest).selectable(true));
        }
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_provider_streams_its_own_way() {
        let c = parse_line(Provider::Claude, r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Olá"}}"#);
        assert_eq!(c.text, "Olá");
        assert!(parse_line(Provider::Claude, "event: content_block_delta").text.is_empty());
        assert!(parse_line(Provider::Claude, r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal"}}"#).refused);
        assert_eq!(
            parse_line(Provider::Claude, r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#).error.as_deref(),
            Some("Overloaded")
        );
        assert_eq!(parse_line(Provider::OpenAi, r#"data: {"choices":[{"delta":{"content":"oi"}}]}"#).text, "oi");
        assert!(parse_line(Provider::OpenAi, "data: [DONE]").done);
        assert_eq!(parse_line(Provider::Gemini, r#"data: {"candidates":[{"content":{"parts":[{"text":"a"},{"text":"b"}]}}]}"#).text, "ab");
        let o = parse_line(Provider::Ollama, r#"{"message":{"role":"assistant","content":"x"},"done":false}"#);
        assert_eq!((o.text.as_str(), o.done), ("x", false));
        assert!(parse_line(Provider::Ollama, r#"{"done":true}"#).done);
        assert_eq!(parse_line(Provider::Ollama, r#"{"error":"model not found"}"#).error.as_deref(), Some("model not found"));
        assert_eq!(parse_line(Provider::OpenAi, "data: not json"), Piece::default());
    }

    #[test]
    fn the_document_goes_out_page_by_page_within_the_limit() {
        let pages = vec![(0, "Introdução".to_owned()), (1, "Capítulo um".to_owned()), (2, "Capítulo dois".to_owned())];
        let (text, cut) = document_text(&pages, 10_000);
        assert!(text.starts_with("=== Page 1 ===\nIntrodução") && text.contains("=== Page 3 ===\nCapítulo dois"));
        assert_eq!(cut, None);
        let (text, cut) = document_text(&pages, 50);
        assert!(text.contains("Page 1") && !text.contains("Page 3"), "{text}");
        assert_eq!(cut, Some(1), "it got the text up to page 1");
    }

    #[test]
    fn requests_carry_the_document_and_the_conversation() {
        let mut s = AssistantSettings {
            keys: ["ka".into(), "ko".into(), "kg".into(), String::new()],
            models: [CLAUDE_MODEL.into(), "gpt-x".into(), "gemini/x?y".into(), "llama".into()],
            ..AssistantSettings::default()
        };
        let history = [Turn { user: true, text: "Resuma".into() }];
        let c = request(&s, Provider::Claude, "SYS", &history);
        let body: Value = serde_json::from_str(c.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["model"], CLAUDE_MODEL);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][0]["content"], "Resuma");
        assert_eq!(body["fallbacks"], "default");
        assert!(c.headers.iter().any(|(k, v)| k == "x-api-key" && v == "ka"));
        assert!(c.headers.iter().any(|(k, v)| k == "anthropic-beta" && v == "server-side-fallback-2026-07-01"));
        // Another Claude model: no fallback (not every model takes it).
        s.models[0] = "claude-haiku-5-5".into();
        let body: Value = serde_json::from_str(request(&s, Provider::Claude, "SYS", &history).body.as_deref().unwrap()).unwrap();
        assert!(body.get("fallbacks").is_none());
        let g = request(&s, Provider::Gemini, "SYS", &history);
        assert!(g.url.ends_with("/models/geminixy:streamGenerateContent?alt=sse"), "the model can't add a path or query: {}", g.url);
        let o = request(&s, Provider::OpenAi, "SYS", &history);
        let body: Value = serde_json::from_str(o.body.as_deref().unwrap()).unwrap();
        assert_eq!((body["messages"][0]["role"].as_str(), body["messages"][1]["content"].as_str()), (Some("system"), Some("Resuma")));
    }

    #[test]
    fn model_lists_keep_the_chat_models() {
        let openai = r#"{"data":[{"id":"gpt-z"},{"id":"o9"},{"id":"text-embedding-x"},{"id":"gpt-z-audio"},{"id":"whisper-1"}]}"#;
        assert_eq!(parse_models(Provider::OpenAi, openai).unwrap(), ["o9", "gpt-z"]);
        let gemini = r#"{"models":[{"name":"models/gem-a","supportedGenerationMethods":["generateContent"]},{"name":"models/emb","supportedGenerationMethods":["embedContent"]}]}"#;
        assert_eq!(parse_models(Provider::Gemini, gemini).unwrap(), ["gem-a"]);
        assert_eq!(parse_models(Provider::Ollama, r#"{"models":[{"name":"llama3:8b"}]}"#).unwrap(), ["llama3:8b"]);
        assert_eq!(parse_models(Provider::Claude, r#"{"error":{"message":"invalid x-api-key"}}"#), Err("invalid x-api-key".into()));
    }

    #[test]
    fn keys_are_kept_only_when_asked() {
        let mut s = AssistantSettings::default();
        s.keys[0] = "secret".into();
        assert!(s.to_json()["keys"].is_null());
        assert_eq!(AssistantSettings::from_json(&s.to_json()).keys[0], "");
        s.remember_keys = true;
        assert_eq!(AssistantSettings::from_json(&s.to_json()).keys[0], "secret");
        // Nonsense in the settings file: the defaults.
        let d = AssistantSettings::from_json(&json!({"provider": 7, "models": "x", "ollama_url": "file:///etc"}));
        assert_eq!(d, AssistantSettings::default());
    }
}
