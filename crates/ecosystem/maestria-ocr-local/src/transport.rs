use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

pub trait OcrTransport: Send + Sync {
    fn post(&self, endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError>;
}

#[derive(Debug, Clone)]
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(1200))
                .redirects(0)
                .build(),
        }
    }
}

impl OcrTransport for UreqTransport {
    fn post(&self, endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        let response = self
            .agent
            .post(endpoint)
            .set("content-type", "application/json")
            .send_bytes(&body)
            .map_err(|error| PortError::Downstream {
                message: format!("OCR request failed: {error}"),
            })?;
        response
            .into_string()
            .map(String::into_bytes)
            .map_err(|error| PortError::Downstream {
                message: format!("read OCR response: {error}"),
            })
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: u8,
    skip_special_tokens: bool,
    images_config: ImagesConfig,
    stream: bool,
}

impl ChatCompletionRequest {
    pub(crate) fn for_image(model: &str, prompt: &str, mime_type: &str, bytes: &[u8]) -> Self {
        Self {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: vec![
                    ChatContent::Text {
                        text: prompt.to_string(),
                    },
                    ChatContent::Image {
                        image_url: ImageUrl {
                            url: format!("data:{mime_type};base64,{}", BASE64.encode(bytes)),
                        },
                    },
                ],
            }],
            temperature: 0,
            skip_special_tokens: false,
            images_config: ImagesConfig {
                image_mode: "gundam",
            },
            stream: false,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: Vec<ChatContent>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ChatContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    Image { image_url: ImageUrl },
}

#[derive(Debug, Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Debug, Serialize)]
struct ImagesConfig {
    image_mode: &'static str,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

impl ChatCompletionResponse {
    pub(crate) fn text(self) -> Option<String> {
        self.choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .filter(|text| !text.trim().is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<String>,
}
