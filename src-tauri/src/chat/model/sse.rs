//! SSE data framing shared by model adapters. Provider JSON remains in each adapter.
//!
//! Events are dispatched only on a blank line. EOF discards unfinished events, as
//! required by the event-stream protocol; it must not commit a partial model reply.

#[derive(Default)]
pub(super) struct SseDecoder {
    line: Vec<u8>,
    data: Option<String>,
    after_cr: bool,
    first_line_read: bool,
}

impl SseDecoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                self.read_line(&mut events);
                self.after_cr = byte == b'\r';
            } else {
                self.line.push(byte);
            }
        }
        events
    }

    fn read_line(&mut self, events: &mut Vec<String>) {
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
                let mut events = decoder.push(&wire.as_bytes()[..split]);
                events.extend(decoder.push(&wire.as_bytes()[split..]));
                assert_eq!(events, ["{\"text\":\n\"你好🌍\"}", "", " padded "]);
            }
            let mut decoder = SseDecoder::default();
            let events: Vec<_> = wire
                .as_bytes()
                .iter()
                .flat_map(|byte| decoder.push(&[*byte]))
                .collect();
            assert_eq!(events, ["{\"text\":\n\"你好🌍\"}", "", " padded "]);
        }
    }

    #[test]
    fn data_is_not_dispatched_until_blank_line() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"data: first\n").is_empty());
        assert!(decoder.push(b": heartbeat\ndata: second\n").is_empty());
        assert_eq!(decoder.push(b"\n"), ["first\nsecond"]);
        assert!(decoder.push(b"event: ping\n\n").is_empty());
    }
}
