// A small Markdown renderer for model replies. It builds React elements, never
// HTML strings, so nothing a model writes can run as code in the page.

import { Fragment, useState, type ReactNode } from "react";

type Block =
  | { kind: "code"; lang: string; text: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "rule" }
  | { kind: "para"; text: string };

export function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = /^\s*(```|~~~)\s*([\w+-]*)/.exec(line);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence[1])) body.push(lines[i++]);
      i++; // closing fence, or the end while still streaming
      blocks.push({ kind: "code", lang: fence[2], text: body.join("\n") });
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      blocks.push({ kind: "rule" });
      i++;
      continue;
    }
    const item = /^\s*([-*+]|\d+[.)])\s+(.*)$/;
    if (item.test(line)) {
      const ordered = /^\s*\d/.test(line);
      const items: string[] = [];
      while (i < lines.length && item.test(lines[i])) {
        let text = item.exec(lines[i])![2];
        i++;
        // Indented continuation lines belong to the item.
        while (i < lines.length && /^\s{2,}\S/.test(lines[i]) && !item.test(lines[i])) text += ` ${lines[i++].trim()}`;
        items.push(text);
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }
    if (line.startsWith(">")) {
      const body: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) body.push(lines[i++].replace(/^>\s?/, ""));
      blocks.push({ kind: "quote", text: body.join("\n") });
      continue;
    }
    const body: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !/^(#{1,6}\s|\s*```|\s*~~~|>)/.test(lines[i]) &&
      !item.test(lines[i])
    ) {
      body.push(lines[i++]);
    }
    blocks.push({ kind: "para", text: body.join("\n") });
  }
  return blocks;
}

/** Inline code, bold, italics and links. Only http(s) and mailto links are kept. */
export function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const pattern = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*|__[^_\n]+__)|(\*[^*\n]+\*|_[^_\n]+_)|(\[[^\]\n]+\]\([^)\s]+\))/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let key = 0;
  while ((m = pattern.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const t = m[0];
    if (m[1]) out.push(<code key={key++}>{t.slice(1, -1)}</code>);
    else if (m[2]) out.push(<strong key={key++}>{inline(t.slice(2, -2))}</strong>);
    else if (m[3]) out.push(<em key={key++}>{inline(t.slice(1, -1))}</em>);
    else {
      const [, label, href] = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(t)!;
      out.push(
        /^(https?:|mailto:)/i.test(href) ? (
          <a key={key++} href={href} target="_blank" rel="noopener noreferrer">
            {label}
          </a>
        ) : (
          label
        ),
      );
    }
    last = m.index + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

function withBreaks(text: string): ReactNode[] {
  return text.split("\n").map((line, i) => (
    <Fragment key={i}>
      {i > 0 && <br />}
      {inline(line)}
    </Fragment>
  ));
}

function CodeBlock({ lang, text }: { lang: string; text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="code">
      <div className="code-head">
        <span>{lang || "code"}</span>
        <button
          className="ghost small"
          onClick={() => {
            navigator.clipboard?.writeText(text).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            });
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre>
        <code>{text}</code>
      </pre>
    </div>
  );
}

export function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      {parseBlocks(text).map((b, i) => {
        switch (b.kind) {
          case "code":
            return <CodeBlock key={i} lang={b.lang} text={b.text} />;
          case "heading": {
            const H = `h${Math.min(b.level + 2, 6)}` as "h3";
            return <H key={i}>{inline(b.text)}</H>;
          }
          case "list": {
            const L = b.ordered ? "ol" : "ul";
            return (
              <L key={i}>
                {b.items.map((it, j) => (
                  <li key={j}>{inline(it)}</li>
                ))}
              </L>
            );
          }
          case "quote":
            return <blockquote key={i}>{withBreaks(b.text)}</blockquote>;
          case "rule":
            return <hr key={i} />;
          case "para":
            return <p key={i}>{withBreaks(b.text)}</p>;
        }
      })}
    </div>
  );
}
