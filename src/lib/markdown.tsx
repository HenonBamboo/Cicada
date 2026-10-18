import { Fragment, type ReactNode } from "react";

const SAFE_LINK = /^(?:https?:\/\/|mailto:)/i;
const INLINE_TOKEN =
  /(`[^`\n]+`|\*\*[^*\n]+\*\*|__[^_\n]+__|~~[^~\n]+~~|\[[^\]\n]+\]\([^\s)]+(?:\s+"[^"]*")?\)|\*[^*\n]+\*|_[^_\n]+_)/g;

function inlineNodes(text: string, prefix = "md"): ReactNode[] {
  const nodes: ReactNode[] = [];
  let cursor = 0;
  let index = 0;
  for (const match of text.matchAll(INLINE_TOKEN)) {
    const start = match.index ?? 0;
    if (start > cursor) nodes.push(text.slice(cursor, start));
    const token = match[0];
    const key = `${prefix}-${index++}`;
    if (token.startsWith("`")) {
      nodes.push(<code key={key}>{token.slice(1, -1)}</code>);
    } else if (token.startsWith("**") || token.startsWith("__")) {
      nodes.push(<strong key={key}>{inlineNodes(token.slice(2, -2), key)}</strong>);
    } else if (token.startsWith("~~")) {
      nodes.push(<del key={key}>{inlineNodes(token.slice(2, -2), key)}</del>);
    } else if (token.startsWith("[")) {
      const parsed = token.match(/^\[([^\]]+)\]\(([^\s)]+)(?:\s+"[^"]*")?\)$/);
      const label = parsed?.[1] ?? token;
      const href = parsed?.[2] ?? "";
      nodes.push(
        SAFE_LINK.test(href) ? (
          <a key={key} href={href} target="_blank" rel="noreferrer">
            {inlineNodes(label, key)}
          </a>
        ) : (
          <Fragment key={key}>{inlineNodes(label, key)}</Fragment>
        ),
      );
    } else {
      nodes.push(<em key={key}>{inlineNodes(token.slice(1, -1), key)}</em>);
    }
    cursor = start + token.length;
  }
  if (cursor < text.length) nodes.push(text.slice(cursor));
  return nodes;
}

// 与渲染组件共用同一套 Markdown 语法；该纯函数不会影响热更新边界。
// eslint-disable-next-line react-refresh/only-export-components
export function markdownToPlainText(text: string) {
  return text
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/(^|\s)(?:#{1,6}|>|[-+*]|\d+[.)])\s+/gm, "$1")
    .replace(/[`*_~]/g, "")
    .trim();
}

export function MarkdownInline({ text }: { text: string }) {
  return <span className="markdown-inline">{inlineNodes(text)}</span>;
}

function inlineLines(lines: string[], prefix: string) {
  return lines.map((line, index) => (
    <Fragment key={`${prefix}-${index}`}>
      {index > 0 && <br />}
      {inlineNodes(line, `${prefix}-${index}`)}
    </Fragment>
  ));
}

function isBlockStart(line: string) {
  return (
    /^\s*$/.test(line) ||
    /^\s*```/.test(line) ||
    /^\s{0,3}#{1,6}\s+/.test(line) ||
    /^\s{0,3}>\s?/.test(line) ||
    /^\s{0,3}[-+*]\s+/.test(line) ||
    /^\s{0,3}\d+[.)]\s+/.test(line) ||
    /^\s{0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line)
  );
}

function tableCells(line: string) {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((cell) => cell.trim());
}

const TABLE_DIVIDER = /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$/;

export function MarkdownBlock({ text }: { text: string }) {
  const source = text
    .replace(/^\uFEFF/, "")
    .replace(/\r\n?/g, "\n")
    .trim();
  if (!source) return null;
  const lines = source.split("\n");
  const blocks: ReactNode[] = [];
  let cursor = 0;

  while (cursor < lines.length) {
    const line = lines[cursor];
    if (!line.trim()) {
      cursor += 1;
      continue;
    }
    const fence = line.match(/^\s*```([^\s`]*)\s*$/);
    if (fence) {
      const code: string[] = [];
      cursor += 1;
      while (cursor < lines.length && !/^\s*```\s*$/.test(lines[cursor])) {
        code.push(lines[cursor++]);
      }
      if (cursor < lines.length) cursor += 1;
      blocks.push(
        <pre key={`b-${blocks.length}`}>
          <code data-language={fence[1] || undefined}>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }
    const heading = line.match(/^\s{0,3}(#{1,6})\s+(.+)$/);
    if (heading) {
      const level = heading[1].length;
      const content = inlineNodes(heading[2], `h-${cursor}`);
      blocks.push(
        level <= 2 ? (
          <h3 key={`b-${blocks.length}`}>{content}</h3>
        ) : (
          <h4 key={`b-${blocks.length}`}>{content}</h4>
        ),
      );
      cursor += 1;
      continue;
    }
    if (/^\s{0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      blocks.push(<hr key={`b-${blocks.length}`} />);
      cursor += 1;
      continue;
    }
    if (line.includes("|") && cursor + 1 < lines.length && TABLE_DIVIDER.test(lines[cursor + 1])) {
      const headers = tableCells(line);
      const rows: string[][] = [];
      cursor += 2;
      while (cursor < lines.length && lines[cursor].includes("|") && lines[cursor].trim()) {
        rows.push(tableCells(lines[cursor++]));
      }
      blocks.push(
        <div className="markdown-table-wrap" key={`b-${blocks.length}`}>
          <table>
            <thead>
              <tr>
                {headers.map((cell, i) => (
                  <th key={i}>{inlineNodes(cell, `th-${i}`)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, r) => (
                <tr key={r}>
                  {headers.map((_, c) => (
                    <td key={c}>{inlineNodes(row[c] ?? "", `td-${r}-${c}`)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }
    if (/^\s{0,3}>\s?/.test(line)) {
      const quote: string[] = [];
      while (cursor < lines.length) {
        const matched = lines[cursor].match(/^\s{0,3}>\s?(.*)$/);
        if (!matched) break;
        quote.push(matched[1]);
        cursor += 1;
      }
      blocks.push(
        <blockquote key={`b-${blocks.length}`}>{inlineLines(quote, `q-${cursor}`)}</blockquote>,
      );
      continue;
    }
    const unordered = line.match(/^\s{0,3}[-+*]\s+(.+)$/);
    const ordered = line.match(/^\s{0,3}\d+[.)]\s+(.+)$/);
    if (unordered || ordered) {
      const items: string[] = [];
      const matcher = unordered ? /^\s{0,3}[-+*]\s+(.+)$/ : /^\s{0,3}\d+[.)]\s+(.+)$/;
      while (cursor < lines.length) {
        const matched = lines[cursor].match(matcher);
        if (!matched) break;
        items.push(matched[1]);
        cursor += 1;
      }
      const children = items.map((item, i) => (
        <li key={i}>{inlineNodes(item, `li-${cursor}-${i}`)}</li>
      ));
      blocks.push(
        unordered ? (
          <ul key={`b-${blocks.length}`}>{children}</ul>
        ) : (
          <ol key={`b-${blocks.length}`}>{children}</ol>
        ),
      );
      continue;
    }

    const paragraph = [line];
    cursor += 1;
    while (cursor < lines.length && !isBlockStart(lines[cursor])) {
      if (
        lines[cursor].includes("|") &&
        cursor + 1 < lines.length &&
        TABLE_DIVIDER.test(lines[cursor + 1])
      )
        break;
      paragraph.push(lines[cursor++]);
    }
    blocks.push(<p key={`b-${blocks.length}`}>{inlineLines(paragraph, `p-${cursor}`)}</p>);
  }

  return <div className="markdown">{blocks}</div>;
}
