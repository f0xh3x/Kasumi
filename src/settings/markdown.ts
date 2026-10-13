// Release notes (GitHub's Markdown) as DOM, for the update notes window and the
// About page's preview. The text is never parsed as HTML: only the subset the
// notes use (headings, lists, quotes, paragraphs; bold, italic, code, links).
import { invoke } from "@tauri-apps/api/core";
import { $ } from "./dom";

/** **bold**, *italic* / _italic_, `code`, [text](url) inside one line. */
function inline(text: string): Node[] {
  const out: Node[] = [];
  const re = /\*\*(.+?)\*\*|`([^`]+)`|\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)|(?<![\w*])[*_](?!\s)(.+?)(?<!\s)[*_](?![\w*])/g;
  let last = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push(document.createTextNode(text.slice(last, m.index)));
    if (m[1] !== undefined) out.push($("strong", "", ...inline(m[1])));
    else if (m[2] !== undefined) out.push($("code", "", m[2]));
    else if (m[3] !== undefined) {
      const a = $("a", "", ...inline(m[3])) as HTMLAnchorElement;
      const url = m[4];
      a.href = url;
      a.addEventListener("click", (e) => {
        e.preventDefault();
        invoke("run_action", { action: "open", target: url }).catch(console.error);
      });
      out.push(a);
    } else out.push($("em", "", ...inline(m[5])));
    last = re.lastIndex;
  }
  if (last < text.length) out.push(document.createTextNode(text.slice(last)));
  return out;
}

/**
 * The notes as DOM. Sections headed "Install" / "Installation" are left out: the
 * release page's how-to-install, which someone reading this in Kasumi has done.
 */
export function renderMarkdown(md: string): HTMLElement {
  const root = $("div", "notes-body");
  const lines: string[] = [];
  let skipping = 0;
  for (const line of md.replace(/\r\n/g, "\n").split("\n")) {
    const h = /^(#{1,6})\s+(.*)$/.exec(line);
    if (h && skipping && h[1].length <= skipping) skipping = 0;
    if (h && !skipping && /^install(ation|ing)?\b/i.test(h[2].trim())) skipping = h[1].length;
    if (!skipping) lines.push(line);
  }
  let list: HTMLElement | null = null;
  let para: string[] = [];
  let quote: string[] = [];
  const flush = () => {
    if (para.length) root.append($("p", "", ...inline(para.join(" "))));
    if (quote.length) root.append($("blockquote", "", ...inline(quote.join(" "))));
    para = [];
    quote = [];
  };
  for (const raw of lines) {
    const line = raw.trimEnd();
    const item = /^\s*[-*+]\s+(.*)$/.exec(line);
    // A list item's wrapped continuation (indented, no marker).
    if (list && !item && /^\s{2,}\S/.test(line)) {
      list.lastElementChild?.append(" ", ...inline(line.trim()));
      continue;
    }
    if (!item) list = null;
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      const level = Math.min(4, heading[1].length + 1);
      root.append($(`h${level}` as "h2", "", ...inline(heading[2])));
    } else if (item) {
      flush();
      if (!list) {
        list = $("ul");
        root.append(list);
      }
      list.append($("li", "", ...inline(item[1])));
    } else if (/^\s*>/.test(line)) {
      if (para.length) flush();
      quote.push(line.replace(/^\s*>\s?/, ""));
    } else if (!line.trim()) {
      flush();
    } else {
      if (quote.length) flush();
      para.push(line.trim());
    }
  }
  flush();
  return root;
}
