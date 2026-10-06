//! Reads `text/event-stream` bodies into events.

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Event {
    /// `message` when the server didn't name it.
    pub event: String,
    pub data: String,
}

#[derive(Default)]
pub(crate) struct Parser {
    buf: Vec<u8>,
    event: String,
    data: Vec<String>,
}

impl Parser {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push(Event {
                        event: match std::mem::take(&mut self.event) {
                            e if e.is_empty() => "message".into(),
                            e => e,
                        },
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                }
                self.event.clear();
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.event = value.to_string(),
                "data" => self.data.push(value.to_string()),
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_split_across_chunks() {
        let mut p = Parser::default();
        assert!(p
            .push(b"event: endpoint\r\ndata: /messages?session")
            .is_empty());
        let events = p.push(b"=1\r\n\r\n: ping\n\ndata: {\"a\":\ndata: 1}\n\n");
        assert_eq!(
            events,
            vec![
                Event {
                    event: "endpoint".into(),
                    data: "/messages?session=1".into()
                },
                Event {
                    event: "message".into(),
                    data: "{\"a\":\n1}".into()
                },
            ]
        );
    }

    #[test]
    fn multibyte_characters_split_across_chunks() {
        let mut p = Parser::default();
        let text = "data: {\"t\":\"21°C\"}\n\n".as_bytes();
        let cut = text.iter().position(|&b| b == 0xC2).unwrap() + 1;
        assert!(p.push(&text[..cut]).is_empty());
        assert_eq!(p.push(&text[cut..])[0].data, "{\"t\":\"21°C\"}");
    }
}
