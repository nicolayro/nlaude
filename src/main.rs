use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use serde::{Deserialize, Serialize};
use std::io::Write;

const REGION: &str = "eu-north-1";

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const GREY: &str = "\x1b[90m";
const PURPLE: &str = "\x1b[95m";
const CYAN: &str = "\x1b[96m";
const RED: &str = "\x1b[91m";

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
            role: "user".to_string(), // APIets konvensjon: tool_result sendes som user-melding
            content: serde_json::json!([{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": result
            }]),
        }
    }
}

#[derive(Debug)]
enum Response {
    Text(String),
    ToolCall {
        name: String,
        id: String,
        input: serde_json::Value,
        raw_block: serde_json::Value, // originalen sendes tilbake til API-et uforandret
    },
}

fn parse_response(raw: &str) -> Result<Response, Box<dyn std::error::Error>> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let content = v["content"].as_array().ok_or("mangler content-array")?;

    // Tool use har prioritet - finn første tool_use-blokk hvis den finnes
    if let Some(block) = content.iter().find(|b| b["type"] == "tool_use") {
        let name = block["name"].as_str().ok_or("mangler name-felt")?.to_string();
        let id = block["id"].as_str().ok_or("mangler id-felt")?.to_string();
        return Ok(Response::ToolCall {
            name,
            id,
            input: block["input"].clone(),
            raw_block: block.clone(),
        });
    }

    // Ellers forvent tekst
    if let Some(block) = content.iter().find(|b| b["type"] == "text") {
        let text = block["text"].as_str().ok_or("mangler text-felt")?.to_string();
        return Ok(Response::Text(text));
    }

    Err("uventet responsstruktur fra Bedrock".into())
}

// Verktøydefinisjon - sendes med i hver request slik at modellen vet hva den kan bruke
fn tools() -> serde_json::Value {
    serde_json::json!([{
        "name": "read_file",
        "description": "Les innholdet i en fil",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Filsti" }
            },
            "required": ["path"]
        }
    }])
}

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
    let body = serde_json::json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": 1024,
        "system": "Du har tilgang til filsystemet via verktøyene dine. Bruk dem når du trenger informasjon fra filer.",
        "tools": tools(),
        "messages": messages
    });
    let body_str = serde_json::to_string(&body)?;

    let mut req = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json");

    // Signer requesten med AWS Signature V4
    req = sign_request(req, &credentials, &url, &body_str)?;
    let req = req.body(body_str);

    let response = req.send().await?.error_for_status()?.text().await?;
    parse_response(&response)
}

fn sign_request(
    mut req: reqwest::RequestBuilder,
    credentials: &aws_credential_types::Credentials,
    url: &str,
    body: &str,
) -> Result<reqwest::RequestBuilder, Box<dyn std::error::Error>> {
    let identity = credentials.clone().into();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(REGION)
        .name("bedrock")
        .time(std::time::SystemTime::now())
        .settings(SigningSettings::default())
        .build()?;

    let signable = SignableRequest::new(
        "POST",
        url,
        std::iter::empty(),
        SignableBody::Bytes(body.as_bytes()),
    )?;

    let (instructions, _) = sign(signable, &signing_params.into())?.into_parts();

    for (name, value) in instructions.headers() {
        req = req.header(name, value);
    }

    Ok(req)
}

const MAX_ITERATIONS: usize = 10;

// Agentic loop: kall prompt i loop til modellen svarer med tekst
async fn run_agent(context: &mut Vec<Message>) -> Result<String, Box<dyn std::error::Error>> {
    for _ in 0..MAX_ITERATIONS {
        match prompt(context).await? {
            Response::Text(text) => return Ok(text),
            Response::ToolCall { name, id, input: args, raw_block } => {
                match name.as_str() {
                    "read_file" => {
                        let path = args["path"].as_str().unwrap_or("");
                        println!("{CYAN}{BOLD}[verktøy]{RESET}{CYAN} read_file({path}){RESET}");
                        let result = std::fs::read_to_string(path)
                            .unwrap_or_else(|e| format!("feil: {e}"));
                        context.push(Message::assistant_tool_use(raw_block));
                        context.push(Message::tool_result(&id, &result));
                    }
                    other => return Err(format!("ukjent verktøy: {other}").into()),
                }
            }
        }
    }

    Err(format!("agent nådde maks iterasjoner ({MAX_ITERATIONS})").into())
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    println!("{PURPLE}{BOLD}nlaude{RESET}");
    println!("{GREY}Skriv 'exit' for å avslutte.{RESET}\n");

    let mut context: Vec<Message> = Vec::new();
    let mut input = String::new();

    loop {
        input.clear();
        print!("{GREY}du:{RESET} ");
        std::io::stdout().flush().unwrap();
        std::io::stdin().read_line(&mut input).unwrap();

        let user_message = input.trim();
        if user_message.is_empty() { continue; }
        if user_message == "exit" { break; }

        context.push(Message::user(user_message));

        match run_agent(&mut context).await {
            Ok(text) => {
                println!("\n{PURPLE}{BOLD}nlaude:{RESET}\n{text}\n");
                context.push(Message::assistant(&text));
            }
            Err(e) => {
                context.pop();
                eprintln!("{RED}feil:{RESET} {e}");
            }
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
        assert!(matches!(result, Response::ToolCall { name, .. } if name == "read_file"));
    }

    #[test]
    fn parse_response_feiler_ved_ugyldig_struktur() {
        let raw = r#"{"message": "feil"}"#;
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
    async fn agent_uses_tool_and_returns_text() {
        dotenvy::dotenv().ok();
        // Be om innholdet i en fil - modellen må bruke read_file for å svare
        let mut messages = vec![Message::user("Hva er navnet på pakken i Cargo.toml?")];
        let response = run_agent(&mut messages).await.unwrap();
        assert!(response.contains("nlaude"));
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
