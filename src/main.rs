use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use serde::{Deserialize, Serialize};

const REGION: &str = "eu-north-1";

#[derive(Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
}

impl Message {
    fn user(text: &str) -> Self {
        Self { role: "user".to_string(), content: text.to_string() }
    }
}

fn parse_response(raw: &str) -> Result<String, Box<dyn std::error::Error>> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let text = v["content"][0]["text"]
        .as_str()
        .ok_or("uventet responsstruktur fra Bedrock")?
        .to_string();
    Ok(text)
}

#[allow(dead_code)]
async fn prompt(messages: &[Message]) -> Result<String, Box<dyn std::error::Error>> {
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

    let body = serde_json::json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": 1024,
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

fn main() {
    println!("nlaude");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_extracts_text() {
        let raw = r#"{"content": [{"type": "text", "text": "4"}]}"#;
        let result = parse_response(raw).unwrap();
        assert_eq!(result, "4");
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
        assert!(!response.is_empty());
    }
}
