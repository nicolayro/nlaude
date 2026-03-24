use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use serde::{Deserialize, Serialize};

const REGION: &str = "eu-north-1";

#[derive(Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: serde_json::Value,
}

impl Message {
    fn user(text: &str) -> Self {
        Self { role: "user".to_string(), content: serde_json::json!(text) }
    }

    fn assistant(text: &str) -> Self {
        Self { role: "assistant".to_string(), content: serde_json::json!(text) }
    }

    fn assistant_tool_use(block: serde_json::Value) -> Self {
        Self {
            role: "assistant".to_string(),
            content: serde_json::json!([block]),
        }
    }

    fn tool_result(tool_use_id: &str, result: &str) -> Self {
        Self {
            role: "user".to_string(),
            content: serde_json::json!([{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": result
            }]),
        }
    }
}

enum ToolName {
    ReadFile,
}

impl TryFrom<&str> for ToolName {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "read_file" => Ok(ToolName::ReadFile),
            other => Err(format!("ukjent verktøy: {other}")),
        }
    }
}

enum Response {
    Text(String),
    ToolCall { name: ToolName, id: String, input: serde_json::Value, raw_block: serde_json::Value },
}

fn parse_response(raw: &str) -> Result<Response, Box<dyn std::error::Error>> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let block = &v["content"][0];

    match block["type"].as_str() {
        Some("text") => {
            let text = block["text"]
                .as_str()
                .ok_or("mangler text-felt")?
                .to_string();
            Ok(Response::Text(text))
        }
        Some("tool_use") => {
            let name = block["name"]
                .as_str()
                .ok_or("mangler name-felt")?;
            let id = block["id"]
                .as_str()
                .ok_or("mangler id-felt")?
                .to_string();
            Ok(Response::ToolCall {
                name: ToolName::try_from(name)?,
                id,
                input: block["input"].clone(),
                raw_block: block.clone(),
            })
        }
        _ => Err("uventet responsstruktur fra Bedrock".into()),
    }
}

// Tool calls er bare JSON-skjemaer vi sender med i requesten
const TOOLS: &str = r#"[{
    "name": "read_file",
    "description": "Les innholdet i en fil",
    "input_schema": {
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Filsti" }
        },
        "required": ["path"]
    }
}]"#;

async fn prompt(messages: &[Message]) -> Result<Response, Box<dyn std::error::Error>> {
    let model = std::env::var("BEDROCK_MODEL")?;
    let profile = std::env::var("AWS_PROFILE").unwrap_or_else(|_| "default".to_string());

    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .profile_name(profile)
        .region(aws_config::Region::new(REGION))
        .load()
        .await;

    let credentials = config.credentials_provider()
        .ok_or("ingen AWS credentials-provider funnet")?
        .provide_credentials().await?;

    let url = format!(
        "https://bedrock-runtime.{REGION}.amazonaws.com/model/{}/invoke",
        urlencoding::encode(&model)
    );

    // Bygg request-body med hele samtalehistorikken og tilgjengelige verktøy
    let tools: serde_json::Value = serde_json::from_str(TOOLS)?;
    let body = serde_json::json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": 1024,
        "tools": tools,
        "messages": messages
    });
    let body_str = serde_json::to_string(&body)?;

    // Signer requesten med AWS Signature V4
    let identity = credentials.into();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(REGION)
        .name("bedrock")
        .time(std::time::SystemTime::now())
        .settings(SigningSettings::default())
        .build()?;

    let signable = SignableRequest::new(
        "POST",
        &url,
        std::iter::empty(),
        SignableBody::Bytes(body_str.as_bytes()),
    )?;

    let (instructions, _) = sign(signable, &signing_params.into())?.into_parts();

    let mut req = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .body(body_str);

    for (name, value) in instructions.headers() {
        req = req.header(name, value);
    }

    let response = req.send().await?.error_for_status()?.text().await?;
    parse_response(&response)
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let mut context: Vec<Message> = Vec::new();
    let mut input = String::new();

    loop {
        input.clear();
        print!("du: ");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
        std::io::stdin().read_line(&mut input).unwrap();

        let user_message = input.trim();
        if user_message.is_empty() { continue; }
        if user_message == "exit" { break; }

        context.push(Message::user(user_message));

        match prompt(&context).await {
            Ok(Response::Text(text)) => {
                println!("nlaude: {text}");
                context.push(Message::assistant(&text));
            }
            Ok(Response::ToolCall { name: ToolName::ReadFile, id, input: args, raw_block }) => {
                let path = args["path"].as_str().unwrap_or("");
                println!("[verktøy] read_file({path})");
                let result = std::fs::read_to_string(path)
                    .unwrap_or_else(|e| format!("feil: {e}"));
                // APIet krever at assistant-meldingen med tool_use ligger før tool_result
                context.push(Message::assistant_tool_use(raw_block));
                context.push(Message::tool_result(&id, &result));
            }
            Err(e) => eprintln!("feil: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_extracts_text() {
        let raw = r#"{"content": [{"type": "text", "text": "4"}]}"#;
        let result = parse_response(raw).unwrap();
        assert!(matches!(result, Response::Text(t) if t == "4"));
    }

    #[test]
    fn parse_response_extracts_tool_call() {
        let raw = r#"{"content": [{"type": "tool_use", "id": "abc", "name": "read_file", "input": {"path": "foo.txt"}}]}"#;
        let result = parse_response(raw).unwrap();
        assert!(matches!(result, Response::ToolCall { name: ToolName::ReadFile, .. }));
    }

    #[test]
    fn parse_response_feiler_ved_ugyldig_struktur() {
        let raw = r#"{"message": "feil"}"#;
        assert!(parse_response(raw).is_err());
    }

    #[test]
    fn parse_response_feiler_ved_ukjent_tool() {
        let raw = r#"{"content": [{"type": "tool_use", "id": "abc", "name": "ukjent_tool", "input": {}}]}"#;
        assert!(parse_response(raw).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn prompt_returns_response() {
        dotenvy::dotenv().ok();
        let messages = [Message::user("Hva er 2 + 2? Svar kun med tallet.")];
        let response = prompt(&messages).await.unwrap();
        assert!(matches!(response, Response::Text(_)));
    }

    #[tokio::test]
    #[ignore]
    async fn context_remembers_previous_messages() {
        dotenvy::dotenv().ok();
        let messages = [
            Message::user("Mitt favorittall er 42. Husk det."),
            Message::assistant("Jeg har notert det! Favorittallet ditt er 42."),
            Message::user("Hva er favorittallet mitt? Svar kun med tallet."),
        ];
        let response = prompt(&messages).await.unwrap();
        assert!(matches!(response, Response::Text(t) if t.contains("42")));
    }
}
