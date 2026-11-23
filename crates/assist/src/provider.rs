//! Where questions go: Anthropic's Messages API, or any service that
//! speaks the `OpenAI` chat-completions shape (a local model, for one).
//! Plain HTTP with JSON; the key is read from an environment variable the
//! user names, never from the settings file.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Effort, Error, Request};

/// Something that answers a [`Request`] with text (JSON, as asked).
pub trait Provider: Send + Sync {
    fn complete(&self, request: &Request) -> Result<String, Error>;
}

/// The Messages API's version this code speaks.
const ANTHROPIC_VERSION: &str = "2026-10-21";
const ANTHROPIC_URL: &str = "https://api.anthropic.com";
/// Models that take a fallback model when they decline (`fallbacks: "default"`), with its beta flag.
const FALLBACK_MODELS: [&str; 5] =
    ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// The most of an error body kept for the message.
const ERROR_BODY: usize = 300;

fn effort(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build();
    config.into()
}

/// Sends `body` to `url` and gives back the JSON answer, or what went wrong.
fn post(url: &str, headers: &[(&str, &str)], body: &Value, timeout: Duration) -> Result<Value, Error> {
    let mut request = agent(timeout).post(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request.send_json(body).map_err(|error| Error::Network(error.to_string()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let text = response.body_mut().read_to_string().unwrap_or_default();
        return Err(Error::Status(status, text.chars().take(ERROR_BODY).collect()));
    }
    response.body_mut().read_json::<Value>().map_err(|error| Error::BadAnswer(error.to_string()))
}

/// Claude, through Anthropic's Messages API.
#[derive(Debug, Clone)]
pub struct Anthropic {
    base_url: String,
    model: String,
    key: String,
}

impl Anthropic {
    /// `base_url`: empty for Anthropic's own service.
    #[must_use]
    pub fn new(base_url: &str, model: &str, key: &str) -> Self {
        let base_url = if base_url.is_empty() { ANTHROPIC_URL } else { base_url };
        Self { base_url: base_url.trim_end_matches('/').to_owned(), model: model.to_owned(), key: key.to_owned() }
    }

    /// The request's body: structured output in the asked shape, effort as asked.
    fn body(&self, request: &Request) -> (Value, bool) {
        let mut body = json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "system": request.system,
            "messages": [{ "role": "user", "content": request.user }],
            "output_config": {
                "effort": effort(request.effort),
                "format": { "type": "json_schema", "schema": request.schema },
            },
        });
        // A model that may decline hands the question to another instead of failing.
        let falls_back = FALLBACK_MODELS.contains(&self.model.as_str());
        if falls_back {
            body["fallbacks"] = json!("default");
        }
        (body, falls_back)
    }
}

impl Provider for Anthropic {
    fn complete(&self, request: &Request) -> Result<String, Error> {
        let (body, falls_back) = self.body(request);
        let mut headers = vec![
            ("x-api-key", self.key.as_str()),
            ("anthropic-version", ANTHROPIC_VERSION),
            ("content-type", "application/json"),
        ];
        if falls_back {
            headers.push(("anthropic-beta", FALLBACK_BETA));
        }
        let answer = post(&format!("{}/v1/messages", self.base_url), &headers, &body, request.timeout)?;
        if answer["stop_reason"] == "refusal" {
            return Err(Error::Refused);
        }
        let blocks = answer["content"].as_array().ok_or_else(|| Error::BadAnswer("no content".into()))?;
        let text: String =
            blocks.iter().filter(|block| block["type"] == "text").filter_map(|block| block["text"].as_str()).collect();
        if text.is_empty() { Err(Error::BadAnswer("no text in the answer".into())) } else { Ok(text) }
    }
}

/// Which field of the request says how long the answer may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tokens {
    /// `max_tokens`: what such services have in common.
    Max,
    /// `max_completion_tokens`: what `OpenAI`'s own service asks for.
    MaxCompletion,
}

/// How a service is told that the answer is JSON, from the strictest down.
/// A service is asked the strictest way first; when it refuses the request,
/// the next way is tried, and the way that worked is kept for the questions
/// that follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// `response_format: json_schema`: the service lets the model write only what fits the schema.
    Schema,
    /// `response_format: json_object`: JSON of any shape.
    Object,
    /// No `response_format`: the shape is only in the instructions.
    Plain,
}

impl Format {
    const fn looser(self) -> Option<Self> {
        match self {
            Self::Schema => Some(Self::Object),
            Self::Object => Some(Self::Plain),
            Self::Plain => None,
        }
    }
}

/// A service with `OpenAI`'s chat-completions shape (`POST {base}/chat/completions`).
#[derive(Debug)]
pub struct OpenAiCompatible {
    base_url: String,
    model: String,
    /// Empty for a service that needs none (a local one).
    key: String,
    tokens: Tokens,
    format: Mutex<Format>,
}

impl OpenAiCompatible {
    #[must_use]
    pub fn new(base_url: &str, model: &str, key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            model: model.to_owned(),
            key: key.to_owned(),
            tokens: Tokens::Max,
            format: Mutex::new(Format::Schema),
        }
    }

    /// The same service, told the answer's length in the field `tokens` names.
    #[must_use]
    pub fn with_tokens(mut self, tokens: Tokens) -> Self {
        self.tokens = tokens;
        self
    }

    fn format(&self) -> Format {
        *self.format.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One request, with the answer's JSON asked for in the way `format` says.
    fn ask(&self, request: &Request, format: Format) -> Result<String, Error> {
        let system = format!("{}\nThe JSON schema of the answer: {}", request.system, request.schema);
        let mut body = json!({
            "model": self.model,
            "messages": [{ "role": "system", "content": system }, { "role": "user", "content": request.user }],
        });
        let length = match self.tokens {
            Tokens::Max => "max_tokens",
            Tokens::MaxCompletion => "max_completion_tokens",
        };
        body[length] = json!(request.max_tokens);
        match format {
            Format::Schema => {
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": { "name": "answer", "strict": true, "schema": request.schema },
                });
            }
            Format::Object => body["response_format"] = json!({ "type": "json_object" }),
            Format::Plain => {}
        }
        let bearer = format!("Bearer {}", self.key);
        let mut headers = vec![("content-type", "application/json")];
        if !self.key.is_empty() {
            headers.push(("authorization", bearer.as_str()));
        }
        let answer = post(&format!("{}/chat/completions", self.base_url), &headers, &body, request.timeout)?;
        answer["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::BadAnswer("no choices[0].message.content".into()))
    }
}

impl Provider for OpenAiCompatible {
    fn complete(&self, request: &Request) -> Result<String, Error> {
        let mut format = self.format();
        // What the service said to the strictest way: the most telling, if no way works.
        let mut refused = None;
        loop {
            match self.ask(request, format) {
                // "Bad request", "unprocessable": the service does not take the request as it is written.
                Err(Error::Status(code @ (400 | 422), body)) => {
                    let said = refused.take().unwrap_or(Error::Status(code, body));
                    let Some(looser) = format.looser() else { return Err(said) };
                    refused = Some(said);
                    format = looser;
                }
                answered => {
                    // Kept only when it worked: a request refused for another reason (a model
                    // the service does not have) must not make the next questions looser.
                    if answered.is_ok() {
                        *self.format.lock().unwrap_or_else(PoisonError::into_inner) = format;
                    }
                    return answered;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use super::*;

    /// One HTTP exchange on 127.0.0.1: the server answers `status` and `body`
    /// after `delay`, and hands back the request's head and body it got.
    fn serve(status: u16, body: &str, delay: Duration) -> (String, mpsc::Receiver<(String, String)>) {
        serve_each(vec![(status, body.to_owned())], delay)
    }

    /// The same for several exchanges, one connection each, answered in order.
    fn serve_each(answers: Vec<(u16, String)>, delay: Duration) -> (String, mpsc::Receiver<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tell, inbox) = mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                let mut sent = vec![0; length];
                reader.read_exact(&mut sent).unwrap();
                let _ = tell.send((head, String::from_utf8(sent).unwrap()));
                std::thread::sleep(delay);
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (url, inbox)
    }

    fn request(timeout: Duration) -> Request {
        Request {
            system: "be brief".into(),
            user: "hello".into(),
            schema: json!({"type": "object"}),
            max_tokens: 100,
            effort: Effort::Low,
            timeout,
        }
    }

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn claude_is_asked_in_the_messages_shape_and_its_text_is_read() {
        let answer = r#"{"content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "{\"a\": 1}"}], "stop_reason": "end_turn"}"#;
        let (url, inbox) = serve(200, answer, Duration::ZERO);
        let claude = Anthropic::new(&url, "claude-opus-5-5", "sk-test");
        assert_eq!(claude.complete(&request(SECOND * 5)).unwrap(), "{\"a\": 1}");
        let (head, body) = inbox.recv().unwrap();
        let head = head.to_ascii_lowercase();
        assert!(head.starts_with("post /v1/messages "), "{head}");
        assert!(head.contains("x-api-key: sk-test") && head.contains("anthropic-version: 2026-10-21"), "{head}");
        assert!(head.contains("anthropic-beta: server-side-fallback-2026-07-01"), "a model that falls back: {head}");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["output_config"]["effort"], "low");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["messages"][0]["content"], "hello");
        assert!(body.get("thinking").is_none() && body.get("temperature").is_none(), "neither is sent: {body}");
    }

    #[test]
    fn a_model_without_fallbacks_is_asked_without_them() {
        let (url, inbox) = serve(200, r#"{"content": [{"type": "text", "text": "x"}]}"#, Duration::ZERO);
        Anthropic::new(&url, "claude-haiku-5-5", "k").complete(&request(SECOND * 5)).unwrap();
        let (head, body) = inbox.recv().unwrap();
        assert!(!head.to_ascii_lowercase().contains("anthropic-beta"));
        assert!(serde_json::from_str::<Value>(&body).unwrap().get("fallbacks").is_none());
    }

    #[test]
    fn errors_are_told_apart() {
        let (url, _inbox) = serve(200, r#"{"content": [], "stop_reason": "refusal"}"#, Duration::ZERO);
        assert_eq!(Anthropic::new(&url, "m", "k").complete(&request(SECOND * 5)), Err(Error::Refused));
        let (url, _inbox) = serve(401, r#"{"error": {"message": "invalid x-api-key"}}"#, Duration::ZERO);
        let Err(Error::Status(401, body)) = Anthropic::new(&url, "m", "k").complete(&request(SECOND * 5)) else {
            panic!("a 401 is a status error")
        };
        assert!(body.contains("invalid x-api-key"));
        let (url, _inbox) = serve(200, "not json", Duration::ZERO);
        assert!(matches!(Anthropic::new(&url, "m", "k").complete(&request(SECOND * 5)), Err(Error::BadAnswer(_))));
        let (url, _inbox) = serve(200, r#"{"content": []}"#, Duration::from_secs(3));
        let started = std::time::Instant::now();
        let slow = Anthropic::new(&url, "m", "k").complete(&request(SECOND));
        assert!(matches!(slow, Err(Error::Network(_))), "{slow:?}");
        assert!(started.elapsed() < Duration::from_secs(3), "the timeout is kept: {:?}", started.elapsed());
        let nobody = Anthropic::new("http://127.0.0.1:9", "m", "k").complete(&request(SECOND));
        assert!(matches!(nobody, Err(Error::Network(_))), "{nobody:?}");
    }

    #[test]
    fn an_openai_compatible_service_is_asked_in_its_shape() {
        let answer = r#"{"choices": [{"message": {"role": "assistant", "content": "{\"a\": 2}"}}]}"#;
        let (url, inbox) = serve(200, answer, Duration::ZERO);
        let local = OpenAiCompatible::new(&format!("{url}/v1/"), "llama", "");
        assert_eq!(local.complete(&request(SECOND * 5)).unwrap(), "{\"a\": 2}");
        let (head, body) = inbox.recv().unwrap();
        assert!(head.to_ascii_lowercase().starts_with("post /v1/chat/completions "), "{head}");
        assert!(!head.to_ascii_lowercase().contains("authorization"), "no key: no header");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["response_format"]["type"], "json_schema", "the strictest way first");
        assert_eq!(body["response_format"]["json_schema"]["schema"], json!({"type": "object"}));
        assert_eq!((body["max_tokens"].as_u64(), body.get("max_completion_tokens")), (Some(100), None));
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("JSON schema"));
        let (url, inbox) = serve(200, answer, Duration::ZERO);
        let openai = OpenAiCompatible::new(&url, "m", "secret-key").with_tokens(Tokens::MaxCompletion);
        openai.complete(&request(SECOND * 5)).unwrap();
        let (head, body) = inbox.recv().unwrap();
        assert!(head.to_ascii_lowercase().contains("authorization: bearer secret-key"));
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!((body["max_completion_tokens"].as_u64(), body.get("max_tokens")), (Some(100), None));
    }

    /// The `response_format.type` of each request the server got, in order ("none" for a request without).
    fn formats(inbox: &mpsc::Receiver<(String, String)>) -> Vec<String> {
        inbox
            .try_iter()
            .map(|(_, body)| {
                let body: Value = serde_json::from_str(&body).unwrap();
                body["response_format"]["type"].as_str().unwrap_or("none").to_owned()
            })
            .collect()
    }

    #[test]
    fn a_service_is_asked_the_way_it_takes_and_that_way_is_kept() {
        let good = (200, r#"{"choices": [{"message": {"content": "{}"}}]}"#.to_owned());
        let no = |why: &str| (400, format!(r#"{{"error": "{why}"}}"#));
        // A service that takes JSON of any shape only: asked twice the first time, once from then on.
        let (url, inbox) = serve_each(
            vec![no("response_format.type must be json_object"), good.clone(), good.clone()],
            Duration::ZERO,
        );
        let service = OpenAiCompatible::new(&url, "m", "");
        assert_eq!(service.complete(&request(SECOND * 5)).unwrap(), "{}");
        assert_eq!(service.complete(&request(SECOND * 5)).unwrap(), "{}");
        assert_eq!(formats(&inbox), ["json_schema", "json_object", "json_object"]);
        // One that takes no response_format at all: three requests, then one.
        let (url, inbox) =
            serve_each(vec![no("unknown field"), no("unknown field"), good.clone(), good.clone()], Duration::ZERO);
        let service = OpenAiCompatible::new(&url, "m", "");
        assert_eq!(service.complete(&request(SECOND * 5)).unwrap(), "{}");
        assert_eq!(service.complete(&request(SECOND * 5)).unwrap(), "{}");
        assert_eq!(formats(&inbox), ["json_schema", "json_object", "none", "none"]);
        // Refused whatever the way (the model is not there): what the service said first is the error,
        // and the next question starts from the strictest way again.
        let (url, inbox) =
            serve_each(vec![no("no model m"), no("second"), (422, "third".to_owned()), good], Duration::ZERO);
        let service = OpenAiCompatible::new(&url, "m", "");
        let Err(Error::Status(400, said)) = service.complete(&request(SECOND * 5)) else { panic!("refused") };
        assert!(said.contains("no model m"), "{said}");
        assert_eq!(service.complete(&request(SECOND * 5)).unwrap(), "{}");
        assert_eq!(formats(&inbox), ["json_schema", "json_object", "none", "json_schema"]);
        // Another failure (the key is wrong) is not a reason to ask again.
        let (url, inbox) = serve_each(vec![(401, "bad key".to_owned())], Duration::ZERO);
        assert!(matches!(
            OpenAiCompatible::new(&url, "m", "k").complete(&request(SECOND * 5)),
            Err(Error::Status(401, _))
        ));
        assert_eq!(formats(&inbox), ["json_schema"]);
    }
}
