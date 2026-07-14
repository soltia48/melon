import fs from "node:fs";
import path from "node:path";
import { Marked } from "marked";
import markedCjkFriendly from "marked-cjk-friendly";

// The published documents (the legal texts and the merchant guide) are authored
// as Markdown in `web/content/` (the single source of truth) and rendered to HTML
// at BUILD time, so the pages are fully static — the runtime image never reads
// them from disk.
const CONTENT_DIR = path.join(process.cwd(), "content");

// CommonMark's emphasis rules were written for languages that put spaces around
// words: a closing `**` only closes if what precedes it is not punctuation. In
// Japanese, `**…します。**したがって…` therefore does NOT close — marked emits the
// asterisks literally. This extension relaxes the flanking rules for CJK text.
// See https://github.com/commonmark/commonmark-spec/issues/650
const marked = new Marked(markedCjkFriendly());

export interface Doc {
  /** The document's top-level `#` heading, used as the page title. */
  title: string;
  /** The body rendered to HTML (the title heading is not repeated). */
  html: string;
}

/** Read one Markdown document from `web/content/` and render it. */
export function renderDoc(file: string): Doc {
  const md = fs.readFileSync(path.join(CONTENT_DIR, file), "utf8");

  // Lift the leading `# Title` out of the body so the page can render it as its
  // own <h1> (and reuse it as the <title>).
  const heading = md.match(/^#[^#].*$/m);
  const title = heading ? heading[0].replace(/^#\s*/, "").trim() : "Melon";
  const body = heading ? md.replace(heading[0], "") : md;

  // Trusted, in-repo content — no user input reaches this renderer.
  const html = marked.parse(body.trimStart(), { async: false });

  // Unclosed emphasis renders as bare `**` on a published legal page, which is
  // easy to miss in review. Fail the build instead of shipping it.
  if (html.includes("**")) {
    throw new Error(
      `${file}: emphasis markers left unrendered (literal "**" in the output)`,
    );
  }

  return { title, html };
}
