// Generate the documentation in docs/: the guides of files/ (markdown) converted to html, and the API reference of
// the Rust crate (cargo doc) in docs/api/.
//
// npm run doc [-- --no-api]

import { marked } from "marked";
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const input = join(root, "files");
const output = join(root, "docs");
const mermaidUrl = "https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs";

/**
 * the front matter (`title`, `children`) and the markdown of a guide
 */
function readGuide(file) {
    const source = readFileSync(join(input, file), "utf8");
    const match = /^---\n([\s\S]*?)\n---\n/.exec(source);
    const meta = { title: null, children: [] };

    if (match != null) {
        let list = null;

        for (const line of match[1].split("\n")) {
            const entry = /^\s+-\s+(.+)$/.exec(line);

            if (entry != null && list != null) {
                list.push(entry[1].trim());
                continue;
            }

            const pair = /^(\w+):\s*(.*)$/.exec(line);

            if (pair != null) {
                list = pair[2] === "" ? (meta[pair[1]] = []) : null;

                if (pair[2] !== "") {
                    meta[pair[1]] = pair[2];
                }
            }
        }
    }

    return { meta, markdown: match == null ? source : source.slice(match[0].length) };
}

/**
 * GitHub style heading anchor
 */
function slug(text) {
    return text
        .toLowerCase()
        .replace(/<[^>]+>/g, "")
        .replace(/[^\p{L}\p{N}\s_-]/gu, "")
        .trim()
        .replace(/\s/g, "-");
}

function escapeHtml(text) {
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

marked.use({
    renderer: {
        heading({ tokens, depth }) {
            const text = this.parser.parseInline(tokens);
            return `<h${depth} id="${slug(text)}">${text}</h${depth}>\n`;
        },
        code({ text, lang }) {
            if (lang === "mermaid") {
                return `<pre class="mermaid">${escapeHtml(text)}</pre>\n`;
            }

            return `<pre><code${lang ? ` class="language-${lang}"` : ""}>${escapeHtml(text)}</code></pre>\n`;
        },
        link({ href, title, tokens }) {
            // the links to the guides point to their html page
            const target = href.replace(/^(\.\/)?([\w.-]+)\.md(#.*)?$/, (_, __, name, hash) => `${name === "index" ? "index" : name}.html${hash ?? ""}`);
            return `<a href="${target}"${title ? ` title="${title}"` : ""}>${this.parser.parseInline(tokens)}</a>`;
        },
    },
});

const style = `
:root { color-scheme: light dark; --fg: #1f2328; --bg: #fff; --muted: #59636e; --border: #d1d9e0; --code: #f6f8fa; --link: #0969da; }
@media (prefers-color-scheme: dark) { :root { --fg: #e6edf3; --bg: #0d1117; --muted: #9198a1; --border: #3d444d; --code: #151b23; --link: #4493f8; } }
* { box-sizing: border-box; }
body { margin: 0; font: 16px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; color: var(--fg); background: var(--bg); }
.layout { display: flex; max-width: 1200px; margin: 0 auto; }
nav { width: 240px; flex-shrink: 0; padding: 24px 16px; border-right: 1px solid var(--border); position: sticky; top: 0; height: 100vh; overflow-y: auto; }
nav a { display: block; padding: 3px 8px; color: var(--fg); text-decoration: none; border-radius: 6px; font-size: 14px; }
nav a:hover { background: var(--code); }
nav a.current { font-weight: 600; background: var(--code); }
nav h2 { font-size: 12px; text-transform: uppercase; color: var(--muted); margin: 16px 8px 4px; }
main { flex: 1; min-width: 0; padding: 24px 40px 64px; }
a { color: var(--link); }
pre { background: var(--code); padding: 12px 16px; border-radius: 6px; overflow-x: auto; font-size: 14px; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 0.9em; }
:not(pre) > code { background: var(--code); padding: 0.15em 0.35em; border-radius: 4px; }
pre.mermaid { background: none; text-align: center; }
table { border-collapse: collapse; display: block; overflow-x: auto; }
th, td { border: 1px solid var(--border); padding: 6px 12px; text-align: left; vertical-align: top; }
hr { border: 0; border-top: 1px solid var(--border); margin: 32px 0 16px; }
@media (max-width: 800px) { .layout { display: block; } nav { position: static; width: auto; height: auto; border-right: 0; border-bottom: 1px solid var(--border); } main { padding: 16px; } }
`;

function page(title, nav, content, hasMermaid) {
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)} - CSS parser (Rust)</title>
<style>${style}</style>
</head>
<body>
<div class="layout">
<nav>${nav}</nav>
<main>
${content}
</main>
</div>
${hasMermaid ? `<script type="module">import mermaid from "${mermaidUrl}"; mermaid.initialize({ startOnLoad: true, theme: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "default" });</script>` : ""}
</body>
</html>
`;
}

// ---- guides

rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });

const files = readdirSync(input).filter((file) => file.endsWith(".md"));
const index = readGuide("index.md");
// the order of the navigation: the children of the index, then the other guides
const order = ["index.md", ...index.meta.children.map((child) => child.replace(/^\.\//, "")), ...files.sort()].filter(
    (file, i, list) => files.includes(file) && list.indexOf(file) === i,
);
const guides = order.map((file) => {
    const { meta, markdown } = readGuide(file);
    const heading = /^#\s+(.+)$/m.exec(markdown)?.[1];

    return { file, html: file.replace(/\.md$/, ".html"), title: meta.title ?? heading ?? file, markdown };
});

for (const guide of guides) {
    const nav =
        `<h2>Guides</h2>` +
        guides
            .map((g) => `<a href="${g.html}"${g === guide ? ' class="current"' : ""}>${escapeHtml(g.file === "index.md" ? "Contents" : g.title)}</a>`)
            .join("") +
        `<h2>API reference</h2><a href="api/rust_css_parser/index.html">Rust crate</a>` +
        `<a href="https://tbela99.github.io/css-parser/docs/modules/node.html">JavaScript</a>`;
    const content = marked.parse(/^#\s/m.test(guide.markdown) ? guide.markdown : `# ${guide.title}\n\n${guide.markdown}`);

    writeFileSync(join(output, guide.html), page(guide.title, nav, content, guide.markdown.includes("```mermaid")));
}

console.log(`${guides.length} guides written to docs/`);

// ---- rust API reference

if (!process.argv.includes("--no-api")) {
    execFileSync("rustup", ["run", "stable", "cargo", "doc", "--no-deps", "--lib", "-p", "rust-css-parser"], { cwd: root, stdio: "inherit" });
    cpSync(join(root, "target", "doc"), join(output, "api"), { recursive: true });
    console.log("Rust API reference written to docs/api/");
}
