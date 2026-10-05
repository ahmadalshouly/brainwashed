//! Tool calls that models write into their answers. BrainWashed gives models
//! no tools, but models trained for tool use (LFM2, Qwen, Hermes, Mistral,
//! Llama 3.1...) sometimes call one anyway, in their own markup. This finds
//! that markup in the streamed text and turns it into structured calls, so
//! people never see raw `<|tool_call_start|>` tokens.

use serde::Serialize;
use serde_json::{Map, Value};

/// A tool call the model made.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolCall {
    /// `call_0`, `call_1`... in the order the reply made them.
    #[serde(default)]
    pub id: String,
    /// Empty when the call couldn't be read.
    pub name: String,
    /// Named arguments; positional ones are under "0", "1"...
    pub arguments: Value,
    /// The model's text, when the call couldn't be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl ToolCall {
    fn unreadable(raw: &str) -> Self {
        ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: Value::Object(Map::new()),
            raw: Some(raw.trim().to_string()),
        }
    }

    /// The question, when the model "calls" a tool to ask the person
    /// something. Shown as ordinary text, since that is all it is.
    pub fn as_question(&self) -> Option<String> {
        const ASKING: &[&str] = &[
            "ask_user",
            "ask",
            "ask_question",
            "ask_clarification",
            "clarify",
            "request_clarification",
        ];
        if !ASKING.contains(&self.name.to_ascii_lowercase().as_str()) {
            return None;
        }
        let args = self.arguments.as_object()?;
        let texts: Vec<&str> = args.values().filter_map(Value::as_str).collect();
        match texts.as_slice() {
            [one] if args.len() == 1 => Some(one.to_string()),
            _ => ["question", "message", "text", "prompt", "query", "0"]
                .iter()
                .find_map(|k| args.get(*k)?.as_str().map(str::to_string)),
        }
    }
}

/// Part of the answer, after tool calls are taken out.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String),
    Call(ToolCall),
}

/// Start and end of a block of tool calls. Without an end, the calls run to
/// the end of the answer.
const MARKERS: &[(&str, Option<&str>)] = &[
    ("<|tool_call_start|>", Some("<|tool_call_end|>")), // LFM2
    ("<tool_call>", Some("</tool_call>")),              // Hermes, Qwen
    ("<｜tool▁calls▁begin｜>", Some("<｜tool▁calls▁end｜>")), // DeepSeek
    ("[TOOL_CALLS]", None),                             // Mistral
    ("<|python_tag|>", None),                           // Llama 3.1
];

/// Takes tool calls out of a streamed answer, piece by piece. Text that
/// might be the start of a marker is held back until it's clear.
#[derive(Debug, Default)]
pub struct ToolCallFilter {
    buf: String,
    /// The marker of the block being read, if inside one.
    inside: Option<usize>,
}

impl ToolCallFilter {
    pub fn push(&mut self, text: &str) -> Vec<Piece> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            if let Some(m) = self.inside {
                let Some(end) = MARKERS[m].1 else { break };
                let Some(pos) = self.buf.find(end) else { break };
                out.extend(parse_block(&self.buf[..pos]).into_iter().map(Piece::Call));
                self.buf.drain(..pos + end.len());
                self.inside = None;
                continue;
            }
            let first = MARKERS
                .iter()
                .enumerate()
                .filter_map(|(i, (start, _))| self.buf.find(start).map(|p| (p, i)))
                .min();
            if let Some((pos, m)) = first {
                if pos > 0 {
                    out.push(Piece::Text(self.buf[..pos].to_string()));
                }
                self.buf.drain(..pos + MARKERS[m].0.len());
                self.inside = Some(m);
                continue;
            }
            let keep = held_back(&self.buf);
            let ready = self.buf.len() - keep;
            if ready > 0 {
                out.push(Piece::Text(self.buf[..ready].to_string()));
                self.buf.drain(..ready);
            }
            break;
        }
        out
    }

    /// Whatever is left when the answer ends.
    pub fn finish(&mut self) -> Vec<Piece> {
        let rest = std::mem::take(&mut self.buf);
        match self.inside.take() {
            Some(_) if rest.trim().is_empty() => vec![],
            Some(_) => parse_block(&rest).into_iter().map(Piece::Call).collect(),
            None if rest.is_empty() => vec![],
            None => vec![Piece::Text(rest)],
        }
    }
}

/// Bytes at the end of `buf` that could be the start of a marker.
fn held_back(buf: &str) -> usize {
    let mut keep = 0;
    for (start, _) in MARKERS {
        for (i, _) in start.char_indices().skip(1) {
            if buf.ends_with(&start[..i]) {
                keep = keep.max(i);
            }
        }
    }
    keep
}

/// Reads the calls in one block, in whichever format the model used.
pub fn parse_block(body: &str) -> Vec<ToolCall> {
    let body = body.trim();
    if body.is_empty() {
        return vec![];
    }
    if body.contains("<｜tool▁call▁begin｜>") {
        return deepseek(body);
    }
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        let calls: Vec<ToolCall> = match &v {
            Value::Array(items) => items.iter().filter_map(from_json).collect(),
            _ => from_json(&v).into_iter().collect(),
        };
        if !calls.is_empty() {
            return calls;
        }
    }
    if let Some(calls) = Python::new(body).calls() {
        return calls;
    }
    vec![ToolCall::unreadable(body)]
}

/// `{"name": ..., "arguments": {...}}`, also as `parameters` or nested
/// under `function`, with arguments as an object or a JSON string.
fn from_json(v: &Value) -> Option<ToolCall> {
    let obj = v.as_object()?;
    let f = obj
        .get("function")
        .and_then(Value::as_object)
        .unwrap_or(obj);
    let name = f.get("name")?.as_str()?.to_string();
    let args = f
        .get("arguments")
        .or_else(|| f.get("parameters"))
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let arguments = match args {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        other => other,
    };
    Some(ToolCall {
        id: String::new(),
        name,
        arguments,
        raw: None,
    })
}

/// `<｜tool▁call▁begin｜>function<｜tool▁sep｜>name\n```json\n{...}\n```<｜tool▁call▁end｜>`
fn deepseek(body: &str) -> Vec<ToolCall> {
    body.split("<｜tool▁call▁begin｜>")
        .skip(1)
        .map(|part| {
            let part = part.split("<｜tool▁call▁end｜>").next().unwrap_or(part);
            let after = part.split("<｜tool▁sep｜>").nth(1).unwrap_or(part);
            let (name, rest) = after.split_once('\n').unwrap_or((after, ""));
            let json = rest
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            match serde_json::from_str(json) {
                Ok(arguments) => ToolCall {
                    id: String::new(),
                    name: name.trim().to_string(),
                    arguments,
                    raw: None,
                },
                Err(_) => ToolCall::unreadable(part),
            }
        })
        .collect()
}

/// Python-style calls, as LFM2 writes them: `[name(a="x", b=2), other()]`.
struct Python<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Python<'a> {
    fn new(s: &'a str) -> Self {
        Python { s, i: 0 }
    }

    fn calls(mut self) -> Option<Vec<ToolCall>> {
        self.ws();
        let bracketed = self.eat('[');
        let mut calls = Vec::new();
        loop {
            self.ws();
            calls.push(self.call()?);
            self.ws();
            if !self.eat(',') {
                break;
            }
        }
        self.ws();
        if bracketed && !self.eat(']') {
            return None;
        }
        self.ws();
        (self.i == self.s.len()).then_some(calls)
    }

    fn call(&mut self) -> Option<ToolCall> {
        let name = self.ident()?;
        self.ws();
        if !self.eat('(') {
            return None;
        }
        let mut args = Map::new();
        let mut n = 0;
        loop {
            self.ws();
            if self.eat(')') {
                break;
            }
            let save = self.i;
            let named = self.ident().filter(|_| {
                self.ws();
                self.eat('=')
            });
            let key = match named {
                Some(k) => k,
                None => {
                    self.i = save;
                    n += 1;
                    (n - 1).to_string()
                }
            };
            self.ws();
            args.insert(key, self.value()?);
            self.ws();
            if !self.eat(',') {
                self.ws();
                if !self.eat(')') {
                    return None;
                }
                break;
            }
        }
        Some(ToolCall {
            id: String::new(),
            name,
            arguments: Value::Object(args),
            raw: None,
        })
    }

    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            '"' | '\'' => self.string().map(Value::String),
            '[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.eat(']') {
                        return Some(Value::Array(items));
                    }
                    items.push(self.value()?);
                    self.ws();
                    if !self.eat(',') {
                        self.ws();
                        return self.eat(']').then_some(Value::Array(items));
                    }
                }
            }
            '{' => {
                self.i += 1;
                let mut map = Map::new();
                loop {
                    self.ws();
                    if self.eat('}') {
                        return Some(Value::Object(map));
                    }
                    let key = match self.value()? {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    self.ws();
                    if !self.eat(':') {
                        return None;
                    }
                    self.ws();
                    map.insert(key, self.value()?);
                    self.ws();
                    if !self.eat(',') {
                        self.ws();
                        return self.eat('}').then_some(Value::Object(map));
                    }
                }
            }
            _ => {
                let start = self.i;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || "+-._".contains(c))
                {
                    self.i += 1;
                }
                let word = &self.s[start..self.i];
                match word {
                    "" => None,
                    "True" | "true" => Some(Value::Bool(true)),
                    "False" | "false" => Some(Value::Bool(false)),
                    "None" | "null" => Some(Value::Null),
                    _ => serde_json::from_str(word).ok(),
                }
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.i += 1;
        let mut out = String::new();
        let mut chars = self.s[self.i..].char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                c if c == quote => {
                    self.i += at + 1;
                    return Some(out);
                }
                '\\' => {
                    let (_, e) = chars.next()?;
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                }
                c => out.push(c),
            }
        }
        None
    }

    fn ident(&mut self) -> Option<String> {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            self.i += self.peek().unwrap().len_utf8();
        }
        let id = &self.s[start..self.i];
        let first = id.chars().next()?;
        (first.is_alphabetic() || first == '_').then(|| id.to_string())
    }

    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += self.peek().unwrap().len_utf8();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Feeds `text` a few characters at a time, as a model streams it.
    fn run(text: &str, step: usize) -> (String, Vec<ToolCall>) {
        let mut f = ToolCallFilter::default();
        let chars: Vec<char> = text.chars().collect();
        let mut pieces = Vec::new();
        for chunk in chars.chunks(step) {
            pieces.extend(f.push(&chunk.iter().collect::<String>()));
        }
        pieces.extend(f.finish());
        let mut shown = String::new();
        let mut calls = Vec::new();
        for p in pieces {
            match p {
                Piece::Text(t) => shown.push_str(&t),
                Piece::Call(c) => calls.push(c),
            }
        }
        (shown, calls)
    }

    #[test]
    fn lfm_call_from_the_screenshot() {
        let text = "Let me provide a clear summary.<|tool_call_start|>[ask_user(\"Would you like a summary, or do you have a specific question?\")]<|tool_call_end|>";
        for step in [1, 3, 7, 1000] {
            let (shown, calls) = run(text, step);
            assert_eq!(shown, "Let me provide a clear summary.");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "ask_user");
            assert_eq!(
                calls[0].as_question().as_deref(),
                Some("Would you like a summary, or do you have a specific question?")
            );
        }
    }

    #[test]
    fn python_style_arguments() {
        let calls = parse_block(
            "[get_weather(city='Berlin', days=3, metric=True, tags=[\"a\", 'b']), noop()]",
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(
            calls[0].arguments,
            json!({"city": "Berlin", "days": 3, "metric": true, "tags": ["a", "b"]})
        );
        assert_eq!(calls[1].arguments, json!({}));
        assert_eq!(calls[0].as_question(), None);
    }

    #[test]
    fn json_formats() {
        let (shown, calls) = run(
            "Checking.\n<tool_call>\n{\"name\": \"search\", \"arguments\": {\"q\": \"rust\"}}\n</tool_call>",
            4,
        );
        assert_eq!(shown, "Checking.\n");
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments, json!({"q": "rust"}));

        let (shown, calls) = run(
            "[TOOL_CALLS][{\"name\": \"a\", \"arguments\": \"{\\\"x\\\": 1}\"}]",
            5,
        );
        assert_eq!(shown, "");
        assert_eq!(calls[0].arguments, json!({"x": 1}));

        let (_, calls) = run(
            "<|python_tag|>{\"name\": \"calc\", \"parameters\": {\"e\": \"2+2\"}}",
            2,
        );
        assert_eq!(calls[0].name, "calc");
    }

    #[test]
    fn deepseek_format() {
        let calls = parse_block(
            "<｜tool▁call▁begin｜>function<｜tool▁sep｜>lookup\n```json\n{\"id\": 7}\n```<｜tool▁call▁end｜>",
        );
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[0].arguments, json!({"id": 7}));
    }

    #[test]
    fn ordinary_text_passes_through() {
        let text = "Use <b>bold</b> and a < b, or [1, 2] and <tool_ maybe.";
        for step in [1, 2, 100] {
            assert_eq!(run(text, step), (text.to_string(), vec![]));
        }
    }

    #[test]
    fn unreadable_or_unfinished_calls() {
        let (shown, calls) = run("Hi<|tool_call_start|>this is not a call", 3);
        assert_eq!(shown, "Hi");
        assert_eq!(calls[0].name, "");
        assert_eq!(calls[0].raw.as_deref(), Some("this is not a call"));
    }

    #[test]
    fn questions_by_key() {
        let call = ToolCall {
            id: String::new(),
            name: "ask_user".into(),
            arguments: json!({"question": "Which file?", "options": ["a", "b"]}),
            raw: None,
        };
        assert_eq!(call.as_question().as_deref(), Some("Which file?"));
    }
}
