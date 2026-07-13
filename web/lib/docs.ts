import fs from "node:fs";
import path from "node:path";
import { marked } from "marked";

// The legal documents are authored as Markdown in `web/content/` (the single
// source of truth) and rendered to HTML at BUILD time, so the published pages are
// fully static — the runtime image never reads them from disk.
const CONTENT_DIR = path.join(process.cwd(), "content");

export interface LegalDoc {
  /** The document's top-level `#` heading, used as the page title. */
  title: string;
  /** The body rendered to HTML (the title heading is not repeated). */
  html: string;
}

/** Read one Markdown document from `web/content/` and render it. */
export function renderLegalDoc(file: string): LegalDoc {
  const md = fs.readFileSync(path.join(CONTENT_DIR, file), "utf8");

  // Lift the leading `# Title` out of the body so the page can render it as its
  // own <h1> (and reuse it as the <title>).
  const heading = md.match(/^#[^#].*$/m);
  const title = heading ? heading[0].replace(/^#\s*/, "").trim() : "Melon";
  const body = heading ? md.replace(heading[0], "") : md;

  return {
    title,
    // Trusted, in-repo content — no user input reaches this renderer.
    html: marked.parse(body.trimStart(), { async: false }),
  };
}
