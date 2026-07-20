use bytes::Bytes;
use futures::{Stream, StreamExt};

use crate::errors::GatewayError;

// Parses raw SSE framing (lines separated by `\n`, events separated by a
// blank line) and yields each event's `data:` payload, trimmed and joined if
// an event spans multiple `data:` lines. `event:`/`id:`/comment lines are
// ignored — both OpenAI's and Anthropic's payloads carry their own `type`
// field, so there's no need to correlate with the SSE `event:` line. The
// terminal `data: [DONE]` sentinel (OpenAI's convention) is swallowed here
// rather than yielded, since it isn't a real chunk.
pub fn sse_data_lines<S>(bytes_stream: S) -> impl Stream<Item = Result<String, GatewayError>> + Send
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    async_stream::try_stream! {
        futures::pin_mut!(bytes_stream);

        let mut buffer = String::new();
        let mut data_lines: Vec<String> = Vec::new();

        while let Some(chunk) = bytes_stream.next().await {
            let chunk = chunk
                .map_err(|e| GatewayError::Provider(format!("stream read failed: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches('\r').to_string();
                buffer.drain(..=pos);

                if line.is_empty() {
                    if let Some(payload) = flush(&mut data_lines) {
                        yield payload;
                    }
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start().to_string());
                }
            }
        }

        if let Some(payload) = flush(&mut data_lines) {
            yield payload;
        }
    }
}

fn flush(data_lines: &mut Vec<String>) -> Option<String> {
    if data_lines.is_empty() {
        return None;
    }
    let payload = data_lines.join("\n");
    data_lines.clear();
    if payload == "[DONE]" {
        None
    } else {
        Some(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(parts: &[&str]) -> impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static {
        let owned: Vec<reqwest::Result<Bytes>> = parts
            .iter()
            .map(|p| Ok(Bytes::from(p.as_bytes().to_vec())))
            .collect();
        futures::stream::iter(owned)
    }

    async fn collect(parts: &[&str]) -> Vec<String> {
        sse_data_lines(chunks(parts))
            .map(|r| r.unwrap())
            .collect()
            .await
    }

    #[tokio::test]
    async fn yields_one_payload_per_event() {
        let out = collect(&["data: {\"a\":1}\n\n", "data: {\"a\":2}\n\n"]).await;
        assert_eq!(out, vec!["{\"a\":1}".to_string(), "{\"a\":2}".to_string()]);
    }

    #[tokio::test]
    async fn splits_events_across_chunk_boundaries() {
        let out = collect(&["data: {\"a", "\":1}\n\n"]).await;
        assert_eq!(out, vec!["{\"a\":1}".to_string()]);
    }

    #[tokio::test]
    async fn swallows_done_sentinel() {
        let out = collect(&["data: {\"a\":1}\n\n", "data: [DONE]\n\n"]).await;
        assert_eq!(out, vec!["{\"a\":1}".to_string()]);
    }

    #[tokio::test]
    async fn ignores_event_and_id_lines() {
        let out = collect(&["event: message_start\nid: 1\ndata: {\"a\":1}\n\n"]).await;
        assert_eq!(out, vec!["{\"a\":1}".to_string()]);
    }

    #[tokio::test]
    async fn flushes_trailing_event_without_blank_line() {
        let out = collect(&["data: {\"a\":1}\n"]).await;
        assert_eq!(out, vec!["{\"a\":1}".to_string()]);
    }
}
