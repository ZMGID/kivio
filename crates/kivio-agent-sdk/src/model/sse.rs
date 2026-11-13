//! SSE data framing shared by model adapters. Provider JSON remains in each adapter.
//!
//! Events are dispatched only on a blank line. EOF discards unfinished events, as
//! required by the event-stream protocol; it must not commit a partial model reply.

use super::{ModelError, ModelErrorKind};
const MAX_PENDING_EVENT_BYTES: usize = 64 * 1024 * 1024;
pub struct SseDecoder {
    limit: usize,
    line: Vec<u8>,
    data: Option<String>,
    after_cr: bool,
    first_line_read: bool,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self {
            limit: MAX_PENDING_EVENT_BYTES,
            line: Vec::new(),
            data: None,
            after_cr: false,
            first_line_read: false,
        }
    }
}
impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, ModelError> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                self.read_line(&mut events)?;
                self.after_cr = byte == b'\r';
            } else {
                self.check_size(self.line.len() + 1)?;
                self.line.push(byte);
            }
        }
        Ok(events)
    }

    fn check_size(&self, bytes: usize) -> Result<(), ModelError> {
        if bytes
            > self
                .limit
                .saturating_sub(self.data.as_ref().map_or(0, String::len))
        {
            return Err(ModelError::with_kind(format!("SSE response event exceeds the pending event limit of {} bytes (64 MiB by default)", self.limit), ModelErrorKind::InvalidRequest));
        }
        Ok(())
    }
    fn read_line(&mut self, events: &mut Vec<String>) -> Result<(), ModelError> {
        // Decode only complete lines, so UTF-8 characters may cross any HTTP
        // chunk boundary. Invalid UTF-8 follows the standard replacement policy.
        let line = String::from_utf8_lossy(&self.line);
        let line = if self.first_line_read {
            line.as_ref()
        } else {
            self.first_line_read = true;
            line.strip_prefix('\u{feff}').unwrap_or(&line)
        };
        if line.is_empty() {
            if let Some(data) = self.data.take() {
                events.push(data);
            }
        } else {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            if field == "data" {
                let value = value.strip_prefix(' ').unwrap_or(value);
                self.check_size(value.len() + usize::from(self.data.is_some()))?;
                if let Some(data) = &mut self.data {
                    data.push('\n');
                    data.push_str(value);
                } else {
                    self.data = Some(value.to_owned());
                }
            }
            // Comments and other fields do not affect model data. Repeated
            // event/id/retry fields and unknown fields are valid SSE input.
        }
        self.line.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SseDecoder;

    #[test]
    fn event_data_survives_every_byte_boundary_and_line_ending() {
        for ending in ["\n", "\r\n", "\r"] {
            let wire = format!(
                "\u{feff}: heartbeat{ending}event: old{ending}event: new{ending}unknown{ending}id: 1{ending}id: 2{ending}data: {{\"text\":{ending}data: \"你好🌍\"}}{ending}{ending}data{ending}{ending}data:  padded {ending}{ending}data: unfinished{ending}"
            );
            // All partitions include splits inside CRLF, BOM, Chinese and emoji.
            for split in 0..=wire.len() {
                let mut decoder = SseDecoder::default();
                let mut events = decoder.push(&wire.as_bytes()[..split]).unwrap();
                events.extend(decoder.push(&wire.as_bytes()[split..]).unwrap());
                assert_eq!(events, ["{\"text\":\n\"你好🌍\"}", "", " padded "]);
            }
            let mut decoder = SseDecoder::default();
            let events: Vec<_> = wire
                .as_bytes()
                .iter()
                .flat_map(|byte| decoder.push(&[*byte]).unwrap())
                .collect();
            assert_eq!(events, ["{\"text\":\n\"你好🌍\"}", "", " padded "]);
        }
    }

    #[test]
    fn pending_event_limit() {
        for input in [
            b"data: 123456789".as_slice(),
            b": heartbeat too long",
            b"event: too long",
        ] {
            let mut decoder = SseDecoder {
                limit: 12,
                ..Default::default()
            };
            let error = decoder.push(input).unwrap_err();
            assert_eq!(error.kind, super::ModelErrorKind::InvalidRequest);
            assert!(!error.is_retryable_stream_error());
            assert!(decoder.line.len() <= 12);
        }
        let mut decoder = SseDecoder {
            limit: 12,
            ..Default::default()
        };
        assert!(decoder.push(b"data: abc\ndata: d\n").unwrap().is_empty());
        assert!(decoder.push(b"data: ef").is_err());
        let mut decoder = SseDecoder {
            limit: 12,
            ..Default::default()
        };
        assert_eq!(decoder.push(b"data: 123456\n\n").unwrap(), ["123456"]);
        assert_eq!(
            decoder.push(b"data: abc\ndata: def\n\n").unwrap(),
            ["abc\ndef"]
        );
    }
    #[test]
    fn data_is_not_dispatched_until_blank_line() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"data: first\n").unwrap().is_empty());
        assert!(decoder
            .push(b": heartbeat\ndata: second\n")
            .unwrap()
            .is_empty());
        assert_eq!(decoder.push(b"\n").unwrap(), ["first\nsecond"]);
        assert!(decoder.push(b"event: ping\n\n").unwrap().is_empty());
    }
}
