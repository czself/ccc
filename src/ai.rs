use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_MODEL: &str = "deepseek-v4-flash";

pub struct AiEditRequest<'a> {
    pub instruction: &'a str,
    pub filename: &'a str,
    pub language: &'a str,
    pub content: &'a str,
    pub history: &'a [AiTurn],
}

pub struct AiChatRequest<'a> {
    pub question: &'a str,
    pub filename: &'a str,
    pub language: &'a str,
    pub content: &'a str,
    pub history: &'a [AiTurn],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiTurn {
    pub role: AiRole,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiRole {
    User,
    Assistant,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Serialize, Deserialize)]
struct AiConfig {
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Clone, Copy)]
enum AiConfigSource {
    ConfigFile,
    TinyvimEnv,
    OpenAiEnv,
}

pub fn edit_current_file(request: AiEditRequest<'_>) -> Result<String, String> {
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: edit_system_prompt().into(),
    }];
    append_history(&mut messages, request.history);
    messages.push(ChatMessage {
        role: "user".into(),
        content: edit_user_prompt(request),
    });

    let content = call_chat_completion(messages)?;
    let edited = strip_code_fence(content.trim());
    if edited.trim().is_empty() {
        return Err("AI returned an empty file".to_string());
    }
    Ok(edited.to_string())
}

pub fn chat(request: AiChatRequest<'_>) -> Result<String, String> {
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: chat_system_prompt().into(),
    }];
    append_history(&mut messages, request.history);
    messages.push(ChatMessage {
        role: "user".into(),
        content: chat_user_prompt(request),
    });
    call_chat_completion(messages)
}

fn append_history(messages: &mut Vec<ChatMessage>, history: &[AiTurn]) {
    let start = history.len().saturating_sub(12);
    for turn in &history[start..] {
        messages.push(ChatMessage {
            role: match turn.role {
                AiRole::User => "user".into(),
                AiRole::Assistant => "assistant".into(),
            },
            content: turn.content.clone(),
        });
    }
}

fn call_chat_completion(messages: Vec<ChatMessage>) -> Result<String, String> {
    let config =
        load_config().ok_or_else(|| "Set AI API key first with Ctrl+E or Ctrl+R".to_string())?;
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

    let body = ChatRequest {
        model: config.model,
        temperature: 0.2,
        messages,
    };

    let body =
        serde_json::to_string(&body).map_err(|e| format!("AI request build failed: {}", e))?;
    let text = send_chat_request(&url, &config.api_key, body)?;
    let parsed: ChatResponse = serde_json::from_str(&text).map_err(|e| {
        format!(
            "AI response parse failed: {}\nResponse body:\n{}",
            e,
            trim_error_body(&text)
        )
    })?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .ok_or_else(|| "AI returned no choices".to_string())?;
    if content.trim().is_empty() {
        return Err("AI returned an empty response".to_string());
    }
    Ok(content)
}

pub fn has_config() -> bool {
    load_config().is_some()
}

pub fn config_summary() -> Result<String, String> {
    let (config, source) = load_config_with_source()
        .ok_or_else(|| "AI is not configured. Press Ctrl+E or Ctrl+R to set it up.".to_string())?;
    Ok(format!(
        "AI provider: OpenAI-compatible\nSource: {}\nBase URL: {}\nModel: {}\nAPI Key: {}",
        source.label(),
        config.base_url,
        config.model,
        mask_api_key(&config.api_key)
    ))
}

pub fn test_current_config() -> Result<String, String> {
    let (config, source) = load_config_with_source()
        .ok_or_else(|| "AI is not configured. Press Ctrl+E or Ctrl+R to set it up.".to_string())?;
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = ChatRequest {
        model: config.model.clone(),
        temperature: 0.0,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "ping".into(),
        }],
    };
    let body = serde_json::to_string(&body).map_err(|e| format!("AI test build failed: {}", e))?;
    let _ = send_chat_request(&url, &config.api_key, body)?;

    Ok(format!(
        "AI config test passed.\nSource: {}\nBase URL: {}\nModel: {}\nAPI Key: {}",
        source.label(),
        config.base_url,
        config.model,
        mask_api_key(&config.api_key)
    ))
}

pub fn is_auth_error_message(message: &str) -> bool {
    message.starts_with("AI auth failed:")
}

pub fn auth_error_detail(message: &str) -> String {
    let config = config_summary().unwrap_or_else(|e| e);
    let key_hint = load_config()
        .map(|config| api_key_health_hint(&config.api_key))
        .unwrap_or_default();
    let config_path = config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|e| e);
    let override_note = if env_overrides_ai_config() {
        "Environment variables are active and override the saved F2 config.\nUpdate or clear TINYVIM_AI_API_KEY / OPENAI_API_KEY, then try again."
    } else {
        "Press F2 to reconfigure Base URL, Model, and API Key.\nThe API Key prompt is hidden; paste the key and press Enter."
    };

    format!(
        "{message}\n\nWhat this means:\nHTTP 401/403 means the AI service was reached, but the API key was rejected.\nThis is usually a wrong, revoked, disabled, truncated, or copied-with-spaces key.\n{key_hint}\nCurrent config:\n{config}\nConfig file:\n{config_path}\n\n{override_note}\n\nDeepSeek defaults:\nBase URL: {base_url}\nModel: {model}",
        base_url = DEFAULT_BASE_URL,
        model = DEFAULT_MODEL
    )
}

fn ai_request_error(error: &str) -> String {
    if error.contains("401") || error.contains("403") {
        if env_overrides_ai_config() {
            format!(
                "AI auth failed: {}. AI key comes from environment variables; update or clear TINYVIM_AI_API_KEY/OPENAI_API_KEY.",
                error
            )
        } else {
            format!(
                "AI auth failed: {}. Press F2 to reconfigure API Key/Base URL/Model.",
                error
            )
        }
    } else {
        format!("AI request failed: {}. Press F2 to reconfigure AI.", error)
    }
}

fn send_chat_request(url: &str, api_key: &str, body: String) -> Result<String, String> {
    let mut response = ureq::post(url)
        .header("Authorization", &format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .config()
        .http_status_as_error(false)
        .build()
        .send(body)
        .map_err(|e| ai_request_error(&e.to_string()))?;

    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("AI response read failed: {}", e))?;

    if (200..300).contains(&status) {
        return Ok(text);
    }

    Err(ai_status_error(status, &text))
}

fn ai_status_error(status: u16, body: &str) -> String {
    let body = trim_error_body(body);
    if status == 401 || status == 403 {
        if env_overrides_ai_config() {
            format!(
                "AI auth failed: HTTP {}.\nServer response:\n{}\n\nAI key comes from environment variables; update or clear TINYVIM_AI_API_KEY/OPENAI_API_KEY.",
                status, body
            )
        } else {
            format!(
                "AI auth failed: HTTP {}.\nServer response:\n{}\n\nPress F2 to reconfigure API Key/Base URL/Model.",
                status, body
            )
        }
    } else {
        format!(
            "AI request failed: HTTP {}.\nServer response:\n{}\n\nPress F2 to reconfigure AI.",
            status, body
        )
    }
}

fn trim_error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "<empty response body>".to_string();
    }
    let mut compact = trimmed.replace("\r\n", "\n");
    const LIMIT: usize = 1600;
    if compact.len() > LIMIT {
        compact.truncate(LIMIT);
        compact.push_str("\n...<truncated>");
    }
    compact
}

pub fn default_base_url() -> &'static str {
    DEFAULT_BASE_URL
}

pub fn default_model() -> &'static str {
    DEFAULT_MODEL
}

pub fn save_config(api_key: &str, base_url: &str, model: &str) -> Result<(), String> {
    let api_key = normalize_api_key(api_key);
    if api_key.is_empty() {
        return Err("AI API key cannot be empty".to_string());
    }
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err("AI base URL cannot be empty".to_string());
    }
    let model = model.trim();
    if model.is_empty() {
        return Err("AI model cannot be empty".to_string());
    }
    let config = AiConfig {
        api_key,
        base_url: base_url.trim_end_matches('/').to_string(),
        model: model.to_string(),
    };
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create AI config directory: {}", e))?;
    }
    let body = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to build AI config: {}", e))?;
    std::fs::write(&path, body)
        .map_err(|e| format!("Failed to save AI config at {}: {}", path.display(), e))?;
    Ok(())
}

pub fn env_overrides_ai_config() -> bool {
    std::env::var("TINYVIM_AI_API_KEY").is_ok() || std::env::var("OPENAI_API_KEY").is_ok()
}

fn load_config() -> Option<AiConfig> {
    load_config_with_source().map(|(config, _)| config)
}

fn load_config_with_source() -> Option<(AiConfig, AiConfigSource)> {
    if let Ok(api_key) = std::env::var("TINYVIM_AI_API_KEY") {
        return Some(AiConfig {
            api_key: normalize_api_key(&api_key),
            base_url: std::env::var("TINYVIM_AI_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
            model: std::env::var("TINYVIM_AI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
        })
        .map(|config| (config, AiConfigSource::TinyvimEnv));
    }
    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        return Some(AiConfig {
            api_key: normalize_api_key(&api_key),
            base_url: std::env::var("TINYVIM_AI_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
            model: std::env::var("TINYVIM_AI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
        })
        .map(|config| (config, AiConfigSource::OpenAiEnv));
    }
    let path = config_path().ok()?;
    let body = std::fs::read_to_string(path).ok()?;
    let mut config: AiConfig = serde_json::from_str(&body).ok()?;
    config.api_key = normalize_api_key(&config.api_key);
    Some((config, AiConfigSource::ConfigFile))
}

fn config_path() -> Result<PathBuf, String> {
    let base = config_base_dir(
        std::env::consts::OS,
        std::env::var("APPDATA").ok(),
        std::env::var("LOCALAPPDATA").ok(),
        std::env::var("USERPROFILE").ok(),
        std::env::var("XDG_CONFIG_HOME").ok(),
        std::env::var("HOME").ok(),
    )
    .or_else(|| std::env::current_dir().ok())
    .ok_or_else(|| "Could not find user config directory".to_string())?;
    Ok(base.join("tinyvim").join("ai.json"))
}

fn config_base_dir(
    os: &str,
    appdata: Option<String>,
    localappdata: Option<String>,
    userprofile: Option<String>,
    xdg_config_home: Option<String>,
    home: Option<String>,
) -> Option<PathBuf> {
    match os {
        "windows" => appdata.or(localappdata).or(userprofile).map(PathBuf::from),
        "macos" => home.map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
        }),
        _ => xdg_config_home
            .map(PathBuf::from)
            .or_else(|| home.map(|home| PathBuf::from(home).join(".config"))),
    }
}

fn edit_system_prompt() -> &'static str {
    "You are TinyVim's code editing assistant. Return only the complete updated file content. Do not include explanations, markdown, diffs, or comments about the change unless they belong in the file."
}

fn edit_user_prompt(request: AiEditRequest<'_>) -> String {
    format!(
        "Instruction:\n{}\n\nFilename: {}\nLanguage: {}\n\nCurrent file content:\n```{}\n{}\n```",
        request.instruction, request.filename, request.language, request.language, request.content
    )
}

fn chat_system_prompt() -> &'static str {
    "You are TinyVim's AI assistant. Answer the user's question directly and concisely. You may explain code, discuss the current file, or answer normal questions. Do not rewrite the file unless the user explicitly asks for a code edit."
}

fn chat_user_prompt(request: AiChatRequest<'_>) -> String {
    format!(
        "Question:\n{}\n\nCurrent file for context:\nFilename: {}\nLanguage: {}\n```{}\n{}\n```",
        request.question, request.filename, request.language, request.language, request.content
    )
}

fn strip_code_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let Some(first_newline) = rest.find('\n') else {
        return text;
    };
    let body = &rest[first_newline + 1..];
    if let Some(end) = body.rfind("```") {
        body[..end].trim_end()
    } else {
        text
    }
}

impl AiConfigSource {
    fn label(self) -> &'static str {
        match self {
            AiConfigSource::ConfigFile => "config file",
            AiConfigSource::TinyvimEnv => "environment variable TINYVIM_AI_API_KEY",
            AiConfigSource::OpenAiEnv => "environment variable OPENAI_API_KEY",
        }
    }
}

fn normalize_api_key(api_key: &str) -> String {
    api_key
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect()
}

pub fn api_key_health_hint(api_key: &str) -> String {
    let key = normalize_api_key(api_key);
    let len = key.chars().count();
    if key.starts_with("sk-") && len < 35 {
        format!(
            "\nKey length warning: this saved key is only {} chars. DeepSeek sk-* keys are commonly longer; it may have been pasted incompletely.\n",
            len
        )
    } else {
        String::new()
    }
}

fn mask_api_key(api_key: &str) -> String {
    let key = normalize_api_key(api_key);
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return "<hidden>".to_string();
    }
    let start: String = chars.iter().take(7).collect();
    let end: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}...{} ({} chars)", start, end, chars.len())
}

#[cfg(test)]
mod tests {
    use super::{
        api_key_health_hint, config_base_dir, mask_api_key, normalize_api_key, strip_code_fence,
    };
    use std::path::PathBuf;

    #[test]
    fn strip_code_fence_removes_language_wrapper() {
        assert_eq!(
            strip_code_fence("```cpp\nint main() {}\n```"),
            "int main() {}"
        );
    }

    #[test]
    fn strip_code_fence_keeps_plain_text() {
        assert_eq!(strip_code_fence("print(1)"), "print(1)");
    }

    #[test]
    fn config_base_dir_uses_windows_appdata_first() {
        assert_eq!(
            config_base_dir(
                "windows",
                Some(r"C:\Users\me\AppData\Roaming".to_string()),
                Some(r"C:\Users\me\AppData\Local".to_string()),
                Some(r"C:\Users\me".to_string()),
                None,
                None,
            ),
            Some(PathBuf::from(r"C:\Users\me\AppData\Roaming"))
        );
    }

    #[test]
    fn config_base_dir_uses_macos_application_support() {
        assert_eq!(
            config_base_dir(
                "macos",
                None,
                None,
                None,
                None,
                Some("/Users/me".to_string()),
            ),
            Some(PathBuf::from("/Users/me/Library/Application Support"))
        );
    }

    #[test]
    fn config_base_dir_uses_linux_xdg_or_home_config() {
        assert_eq!(
            config_base_dir(
                "linux",
                None,
                None,
                None,
                Some("/home/me/.config-custom".to_string()),
                Some("/home/me".to_string()),
            ),
            Some(PathBuf::from("/home/me/.config-custom"))
        );
        assert_eq!(
            config_base_dir(
                "linux",
                None,
                None,
                None,
                None,
                Some("/home/me".to_string()),
            ),
            Some(PathBuf::from("/home/me/.config"))
        );
    }

    #[test]
    fn normalize_api_key_removes_pasted_whitespace() {
        assert_eq!(normalize_api_key(" sk-abc\r\n\t123 "), "sk-abc123");
    }

    #[test]
    fn mask_api_key_keeps_only_short_prefix_suffix_and_length() {
        assert_eq!(
            mask_api_key("sk-abcdefghijklmnopqrstuvwxyz123456"),
            "sk-abcd...3456 (35 chars)"
        );
    }

    #[test]
    fn api_key_health_hint_warns_about_short_deepseek_key() {
        assert!(api_key_health_hint("sk-short").contains("only 8 chars"));
        assert!(api_key_health_hint("sk-abcdefghijklmnopqrstuvwxyz123456").is_empty());
    }
}
