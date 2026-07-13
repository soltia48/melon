import Link from "next/link";

/**
 * Shell for a published legal document (利用規約 / 加盟店規約). The body arrives as
 * HTML already rendered from the in-repo Markdown at build time.
 */
export function DocPage({ title, html }: { title: string; html: string }) {
  return (
    <div className="doc-page">
      <header className="doc-top">
        <Link className="doc-brand" href="/">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/melon-logo.png" alt="" width={26} height={26} />
          Melon
        </Link>
        <nav className="doc-nav">
          <Link href="/terms">利用規約</Link>
          <Link href="/merchant-terms">加盟店規約</Link>
        </nav>
      </header>

      <main className="doc">
        <h1>{title}</h1>
        {/* Trusted, in-repo Markdown rendered at build time — no user input. */}
        <div dangerouslySetInnerHTML={{ __html: html }} />
      </main>

      <footer className="doc-foot">
        <Link href="/">← Melon トップへ</Link>
        <span>© 2026 KIRISHIKI Yudai</span>
      </footer>
    </div>
  );
}
