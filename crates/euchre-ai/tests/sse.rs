//! The talk transports: the byte-level SSE parser split at every byte of a
//! recorded stream, both upstream decoders, the request bodies, and the reqwest
//! clients against a local wiremock server (no real network, no real key).

use euchre_ai::model::{ModelParams, ProviderName, model_params};
use euchre_ai::prompt::SystemLayerMessage;
use euchre_ai::talk::{
    AnthropicDecoder, AnthropicTalk, OpenRouterDecoder, OpenRouterTalk, SseEvent, SseParser,
    TalkChunk, TalkClient, TalkError, TalkRequest, anthropic_body, decode_sse, openrouter_body,
};
use euchre_ai::types::DecideUsage;
use futures::StreamExt;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A recorded OpenRouter stream: a keep-alive comment, a role-only chunk,
/// multi-byte text (a typographic apostrophe and an astral character), a
/// usage-only chunk, then `[DONE]` and a trailing chunk that must be ignored.
const OPENROUTER: &str = concat!(
    ": OPENROUTER PROCESSING\n\n",
    "data: {\"id\":\"gen-1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
    "data: {\"id\":\"gen-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"That’s \"}}]}\n\n",
    ": OPENROUTER PROCESSING\n\n",
    "data: {\"id\":\"gen-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a fine 🂡 trick.\"}}]}\r\n\r\n",
    "data: {\"id\":\"gen-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"id\":\"gen-1\",\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":9,\"prompt_tokens_details\":{\"cached_tokens\":64}}}\n\n",
    "data: [DONE]\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"after done\"}}]}\n\n",
);

/// A recorded Anthropic Messages stream.
const ANTHROPIC: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":300,\"cache_creation_input_tokens\":4096,\"cache_read_input_tokens\":0,\"output_tokens\":1}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: ping\n",
    "data: {\"type\": \"ping\"}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Nice \"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hidden\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hand, y’all.\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

fn openrouter_expected() -> Vec<Result<TalkChunk, TalkError>> {
    vec![
        Ok(TalkChunk::Text("That’s ".to_owned())),
        Ok(TalkChunk::Text("a fine 🂡 trick.".to_owned())),
        Ok(TalkChunk::Usage(DecideUsage {
            input_tokens: 120,
            output_tokens: 9,
            cache_read_tokens: 64,
            cache_write_tokens: 0,
        })),
    ]
}

fn anthropic_expected() -> Vec<Result<TalkChunk, TalkError>> {
    vec![
        Ok(TalkChunk::Text("Nice ".to_owned())),
        Ok(TalkChunk::Text("hand, y’all.".to_owned())),
        Ok(TalkChunk::Usage(DecideUsage {
            input_tokens: 300,
            output_tokens: 7,
            cache_read_tokens: 0,
            cache_write_tokens: 4096,
        })),
    ]
}

/* -------------------------------------------------------------------------- */
/* Chunk boundaries                                                           */
/* -------------------------------------------------------------------------- */

#[test]
fn openrouter_stream_decodes_identically_split_at_every_byte() {
    let bytes = OPENROUTER.as_bytes();
    let expected = openrouter_expected();
    assert_eq!(decode_sse(&mut OpenRouterDecoder, &[bytes]), expected);
    for i in 0..=bytes.len() {
        let (a, b) = bytes.split_at(i);
        assert_eq!(
            decode_sse(&mut OpenRouterDecoder, &[a, b]),
            expected,
            "split at {i}"
        );
    }
    let singles: Vec<&[u8]> = bytes.chunks(1).collect();
    assert_eq!(decode_sse(&mut OpenRouterDecoder, &singles), expected);
}

#[test]
fn anthropic_stream_decodes_identically_split_at_every_byte() {
    let bytes = ANTHROPIC.as_bytes();
    let expected = anthropic_expected();
    assert_eq!(
        decode_sse(&mut AnthropicDecoder::default(), &[bytes]),
        expected
    );
    for i in 0..=bytes.len() {
        let (a, b) = bytes.split_at(i);
        assert_eq!(
            decode_sse(&mut AnthropicDecoder::default(), &[a, b]),
            expected,
            "split at {i}"
        );
    }
    let singles: Vec<&[u8]> = bytes.chunks(1).collect();
    assert_eq!(
        decode_sse(&mut AnthropicDecoder::default(), &singles),
        expected
    );
}

#[test]
fn every_three_way_split_of_a_short_stream_agrees() {
    let bytes = "data: {\"choices\":[{\"delta\":{\"content\":\"’ok\"}}]}\r\n\r\ndata: [DONE]\n\n"
        .as_bytes();
    let expected = vec![Ok(TalkChunk::Text("’ok".to_owned()))];
    for i in 0..=bytes.len() {
        for j in i..=bytes.len() {
            let pieces = [&bytes[..i], &bytes[i..j], &bytes[j..]];
            assert_eq!(
                decode_sse(&mut OpenRouterDecoder, &pieces),
                expected,
                "{i}/{j}"
            );
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Parser                                                                     */
/* -------------------------------------------------------------------------- */

fn ev(event: Option<&str>, data: &str) -> SseEvent {
    SseEvent {
        event: event.map(str::to_owned),
        data: data.to_owned(),
    }
}

#[test]
fn parser_fields_comments_and_multiline_data() {
    let mut p = SseParser::new();
    let out = p.push(b": comment\nevent: a\ndata: one\ndata:two\nid: 7\nretry: 10\nunknown\n\n");
    assert_eq!(out, vec![ev(Some("a"), "one\ntwo")]);
    // `data:` with one leading space stripped, only one.
    assert_eq!(p.push(b"data:  spaced\n\n"), vec![ev(None, " spaced")]);
    // A blank line with nothing pending dispatches nothing; the event name resets.
    assert!(p.push(b"\n\n").is_empty());
    assert_eq!(p.push(b"data\n\n"), vec![ev(None, "")]);
    // An event field alone never dispatches.
    assert!(p.push(b"event: lonely\n\n").is_empty());
    assert_eq!(p.push(b"data: x\n\n"), vec![ev(None, "x")]);
}

#[test]
fn parser_handles_crlf_and_flushes_at_eof() {
    let mut p = SseParser::new();
    assert_eq!(p.push(b"data: a\r\n\r\n"), vec![ev(None, "a")]);
    // An unterminated final event is flushed by finish (a deviation from the
    // browser EventSource, which drops it).
    assert!(p.push(b"data: tail").is_empty());
    assert_eq!(p.finish(), vec![ev(None, "tail")]);
    let mut p = SseParser::new();
    assert!(p.push(b"data: pending\n").is_empty());
    assert_eq!(p.finish(), vec![ev(None, "pending")]);
    assert!(SseParser::new().finish().is_empty());
}

#[test]
fn parser_never_splits_a_multibyte_character() {
    let text = "data: ’🂡\n\n".as_bytes();
    for i in 0..=text.len() {
        let mut p = SseParser::new();
        let mut out = p.push(&text[..i]);
        out.extend(p.push(&text[i..]));
        assert_eq!(out, vec![ev(None, "’🂡")], "split at {i}");
    }
}

/* -------------------------------------------------------------------------- */
/* Decoders                                                                   */
/* -------------------------------------------------------------------------- */

#[test]
fn openrouter_in_band_errors_end_the_stream() {
    let s = b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: {\"error\":{\"message\":\"Provider returned error\",\"code\":502}}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"never\"}}]}\n\n";
    assert_eq!(
        decode_sse(&mut OpenRouterDecoder, &[s]),
        vec![
            Ok(TalkChunk::Text("Hi".to_owned())),
            Err(TalkError::Stream("Provider returned error".to_owned())),
        ]
    );
    let bad = b"data: {not json\n\n";
    assert_eq!(
        decode_sse(&mut OpenRouterDecoder, &[bad]),
        vec![Err(TalkError::Stream("invalid JSON chunk".to_owned()))]
    );
    // No [DONE]: the body simply ends.
    let s = b"data: {\"choices\":[{\"delta\":{\"content\":\"end\"}}]}\n\n";
    assert_eq!(
        decode_sse(&mut OpenRouterDecoder, &[s]),
        vec![Ok(TalkChunk::Text("end".to_owned()))]
    );
}

#[test]
fn anthropic_errors_and_eof_without_message_stop() {
    let s = b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
    assert_eq!(
        decode_sse(&mut AnthropicDecoder::default(), &[s]),
        vec![Err(TalkError::Stream("Overloaded".to_owned()))]
    );
    // Usage still flushes at EOF when message_stop never arrives.
    let cut = ANTHROPIC
        .split("event: message_stop")
        .next()
        .unwrap_or_default();
    assert_eq!(
        decode_sse(&mut AnthropicDecoder::default(), &[cut.as_bytes()]),
        anthropic_expected()
    );
}

#[test]
fn error_display_strings() {
    assert_eq!(
        TalkError::Http { status: 429 }.to_string(),
        "AI_APICallError: HTTP 429"
    );
    assert_eq!(
        TalkError::Aborted.to_string(),
        "AbortError: This operation was aborted"
    );
    assert_eq!(
        TalkError::Transport("connect".to_owned()).to_string(),
        "TypeError: fetch failed (connect)"
    );
}

/* -------------------------------------------------------------------------- */
/* Request bodies                                                             */
/* -------------------------------------------------------------------------- */

fn talk_request(model: &str, params: ModelParams) -> TalkRequest {
    TalkRequest {
        model: model.to_owned(),
        system: vec![
            SystemLayerMessage {
                content: "persona".to_owned(),
                cache: true,
            },
            SystemLayerMessage {
                content: "line".to_owned(),
                cache: false,
            },
        ],
        user: "SITUATION=trick_won".to_owned(),
        max_output_tokens: 60,
        params,
    }
}

#[test]
fn openrouter_body_shape() {
    let mut params = model_params("anthropic/claude-haiku-4.5", 0.7, ProviderName::OpenRouter);
    params.temperature = Some(0.7);
    let body = openrouter_body(&talk_request("anthropic/claude-haiku-4.5", params));
    assert_eq!(
        body,
        json!({
            "model": "anthropic/claude-haiku-4.5",
            "messages": [
                { "role": "system", "content": [{
                    "type": "text",
                    "text": "persona",
                    "cache_control": { "type": "ephemeral", "ttl": "1h" }
                }] },
                { "role": "system", "content": [{ "type": "text", "text": "line" }] },
                { "role": "user", "content": "SITUATION=trick_won" }
            ],
            "max_tokens": 60,
            "temperature": 0.7,
            "reasoning": { "effort": "low" },
            "stream": true
        })
    );
    // A no-sampling model sends no temperature at all.
    let params = ModelParams {
        temperature: None,
        provider_options: Map::new(),
    };
    let body = openrouter_body(&talk_request("m", params));
    assert!(body.get("temperature").is_none());
}

#[test]
fn anthropic_body_shape() {
    let params = ModelParams {
        temperature: Some(0.5),
        provider_options:
            json!({ "anthropic": { "thinking": { "type": "disabled" }, "effort": "low" } })
                .as_object()
                .cloned()
                .unwrap_or_default(),
    };
    let body = anthropic_body(&talk_request("claude-haiku-4-5", params));
    assert_eq!(
        body,
        json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 60,
            "system": [
                { "type": "text", "text": "persona", "cache_control": { "type": "ephemeral", "ttl": "1h" } },
                { "type": "text", "text": "line" }
            ],
            "messages": [{ "role": "user", "content": "SITUATION=trick_won" }],
            "temperature": 0.5,
            "thinking": { "type": "disabled" },
            "output_config": { "effort": "low" },
            "stream": true
        })
    );
}

/* -------------------------------------------------------------------------- */
/* reqwest clients                                                            */
/* -------------------------------------------------------------------------- */

async fn collect(
    client: &dyn TalkClient,
    request: TalkRequest,
) -> Result<Vec<Result<TalkChunk, TalkError>>, TalkError> {
    let stream = client.stream(request, CancellationToken::new()).await?;
    Ok(stream.collect().await)
}

fn plain() -> ModelParams {
    ModelParams {
        temperature: None,
        provider_options: Map::new(),
    }
}

#[tokio::test]
async fn openrouter_talk_streams_from_the_server() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .and(header("authorization", "Bearer test"))
        .and(header("accept", "text/event-stream"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(OPENROUTER, "text/event-stream"))
        .mount(&server)
        .await;
    let client = OpenRouterTalk::new(
        "test".to_owned(),
        Some(format!("{}/api/v1/", server.uri())),
        None,
    );
    assert_eq!(
        client.url(),
        format!("{}/api/v1/chat/completions", server.uri())
    );
    let request = talk_request("m", plain());
    let chunks = collect(&client, request.clone()).await.unwrap();
    assert_eq!(chunks, openrouter_expected());
    let sent = server.received_requests().await.unwrap_or_default();
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(body, openrouter_body(&request));
}

#[tokio::test]
async fn anthropic_talk_streams_from_the_server() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ANTHROPIC, "text/event-stream"))
        .mount(&server)
        .await;
    let client = AnthropicTalk::new(
        "test".to_owned(),
        Some(format!("{}/v1", server.uri())),
        None,
    );
    let chunks = collect(&client, talk_request("claude-haiku-4-5", plain()))
        .await
        .unwrap();
    assert_eq!(chunks, anthropic_expected());
}

#[tokio::test]
async fn non_success_statuses_fail_before_streaming_and_drop_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_string("private key and request data"))
        .mount(&server)
        .await;
    let client = OpenRouterTalk::new("test".to_owned(), Some(server.uri()), None);
    let err = collect(&client, talk_request("m", plain()))
        .await
        .unwrap_err();
    assert_eq!(err, TalkError::Http { status: 429 });
    assert!(!format!("{err:?}").contains("private key"));
}

#[tokio::test]
async fn a_cancelled_request_is_never_sent_and_a_cancel_in_flight_aborts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(OPENROUTER, "text/event-stream")
                .set_delay(std::time::Duration::from_secs(30)),
        )
        .mount(&server)
        .await;
    let client = OpenRouterTalk::new("test".to_owned(), Some(server.uri()), None);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = client
        .stream(talk_request("m", plain()), cancel)
        .await
        .err();
    assert_eq!(err, Some(TalkError::Aborted));
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );

    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        c2.cancel();
    });
    let err = client
        .stream(talk_request("m", plain()), cancel)
        .await
        .err();
    assert_eq!(err, Some(TalkError::Aborted));
}

#[tokio::test]
async fn an_unreachable_server_is_a_transport_error() {
    // Bind then drop a listener so the port is closed.
    let addr = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let client = OpenRouterTalk::new("test".to_owned(), Some(format!("http://{addr}")), None);
    let err = collect(&client, talk_request("m", plain()))
        .await
        .unwrap_err();
    assert!(matches!(err, TalkError::Transport(_)), "{err:?}");
}
