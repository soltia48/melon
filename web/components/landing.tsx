import Link from "next/link";
import type { LpCopy } from "@/lib/lp-copy";

const CONTACT = "https://unknowntech.jp/contact";
const GITHUB = "https://github.com/soltia48/melon";
const DL_DESKTOP = "https://github.com/soltia48/melon/releases";
const DL_ANDROID = "https://github.com/soltia48/MelonTerminal-Android/releases";

/** Slashed circle — marks the things a user does NOT have to get hold of. */
function NoIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M6 18L18 6" strokeLinecap="round" />
    </svg>
  );
}

/**
 * The landing page, in whichever language it is handed. `/` renders it with the
 * Japanese copy and `/en` with the English; the markup exists once so the two
 * cannot drift apart.
 */
export function Landing({ c }: { c: LpCopy }) {
  return (
    <div className="lp" lang={c.locale}>
      <nav className="nav">
        <div className="wrap">
          <a className="brand" href="#top">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src="/melon-logo.png" alt="" /> Melon
          </a>
          <div className="links">
            <a href="#features">{c.nav.features}</a>
            <a href="#how">{c.nav.how}</a>
            <a href="#terminals">{c.nav.terminals}</a>
          </div>
          <div className="spacer" />
          <div className="cta-min">
            <Link
              className="lang"
              href={c.other.href}
              hrefLang={c.other.hreflang}
              lang={c.other.hreflang}
            >
              {c.other.label}
            </Link>
            <a className="btn btn-ghost btn-sm" href={CONTACT}>
              {c.nav.contact}
            </a>
            <Link className="btn btn-primary btn-sm" href="/merchant">
              {c.nav.portal}
            </Link>
          </div>
        </div>
      </nav>

      <header className="hero" id="top">
        <div className="wrap">
          <div>
            <div className="eyebrow anim d1">{c.hero.eyebrow}</div>
            <h1 className="anim d2">{c.hero.h1}</h1>
            <p className="lead anim d3">{c.hero.lead}</p>
            <ul className="nofuss anim d3">
              <li>
                <NoIcon />
                {c.hero.noApp}
              </li>
              <li>
                <NoIcon />
                {c.hero.noCard}
              </li>
            </ul>
            <div className="actions anim d4">
              <Link className="btn btn-primary" href="/merchant">
                {c.hero.portal} <span className="arw">→</span>
              </Link>
              <a className="btn btn-ghost" href={CONTACT}>
                {c.hero.contact}
              </a>
            </div>
            <p className="microcopy anim d4">{c.hero.microcopy}</p>
          </div>

          <div className="card-hero anim d3" aria-hidden="true">
            <span className="ping" />
            <div className="badge">
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img src="/melon-logo.png" alt="Melon" />
            </div>
            <div className="tapline">{c.hero.tapline}</div>
            <div className="tapsub">{c.hero.tapsub}</div>
            <div className="chips">
              <div className="chip">
                <div className="k">{c.hero.balance}</div>
                <div className="v">¥65,535</div>
              </div>
              <div className="chip">
                <div className="k">{c.hero.expiry}</div>
                <div className="v mono">2027-01-12</div>
              </div>
            </div>
          </div>
        </div>
      </header>

      <section className="values">
        <div className="wrap">
          <div className="value">
            <div className="vic">
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M7 8.5c2.4 1.8 2.4 5.2 0 7M11 5.5c4 3 4 9.5 0 12.5M15 3c5.4 4 5.4 13 0 17" />
              </svg>
            </div>
            <div>
              <h3>{c.values.tap.t}</h3>
              <p>{c.values.tap.b}</p>
            </div>
          </div>
          <div className="value">
            <div className="vic">
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <rect x="3" y="6" width="18" height="12" rx="2" />
                <path d="M3 10h18M7 14h4" />
              </svg>
            </div>
            <div>
              <h3>{c.values.noCard.t}</h3>
              <p>{c.values.noCard.b}</p>
            </div>
          </div>
          <div className="value">
            <div className="vic">
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <circle cx="8" cy="8" r="4.2" />
                <path d="M11 11l7 7M15.5 17.5l2-2M13.5 19.5l2-2" />
              </svg>
            </div>
            <div>
              <h3>{c.values.merchant.t}</h3>
              <p>{c.values.merchant.b}</p>
            </div>
          </div>
          {/* 規約 第4条: 対応する FeliCa は当社が指定したものに限られる。「どの
              FeliCa でも使える」と読まれないよう、手軽さの訴求には必ずこの注記を添える。 */}
          <p className="vnote">{c.values.note}</p>
        </div>
      </section>

      <section id="features">
        <div className="wrap">
          <div className="sec-head">
            <div className="eyebrow">{c.features.eyebrow}</div>
            <h2>{c.features.h2}</h2>
            <p>{c.features.lead}</p>
          </div>
          <div className="grid">
            <article className="feature">
              <div className="ic">
                <svg
                  width="22"
                  height="22"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M12 3l7 3v5c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z" />
                  <path d="M9.2 12l2 2 3.6-3.8" />
                </svg>
              </div>
              <h3>{c.features.identity.t}</h3>
              <p>{c.features.identity.b}</p>
            </article>
            <article className="feature">
              <div className="ic">
                <svg
                  width="22"
                  height="22"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M6 3h9l4 4v14H6z" />
                  <path d="M9 8h5M9 12h7M9 16h7" />
                </svg>
              </div>
              <h3>{c.features.ledger.t}</h3>
              <p>{c.features.ledger.b}</p>
            </article>
            <article className="feature">
              <div className="ic">
                <svg
                  width="22"
                  height="22"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <circle cx="12" cy="13" r="8" />
                  <path d="M12 9v4l2.5 2M9 2h6" />
                </svg>
              </div>
              <h3>{c.features.expiry.t}</h3>
              <p>{c.features.expiry.b}</p>
            </article>
            <article className="feature">
              <div className="ic">
                <svg
                  width="22"
                  height="22"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="3" y="4" width="18" height="16" rx="2" />
                  <path d="M3 9h18M9 9v11" />
                </svg>
              </div>
              <h3>{c.features.ops.t}</h3>
              <p>{c.features.ops.b}</p>
            </article>
          </div>
        </div>
      </section>

      <section id="how" className="alt">
        <div className="wrap">
          <div className="sec-head">
            <div className="eyebrow">{c.how.eyebrow}</div>
            <h2>{c.how.h2}</h2>
            <p>{c.how.lead}</p>
          </div>
          <div className="steps">
            <div className="step">
              <div className="n" />
              <h3>{c.how.tap.t}</h3>
              <p>{c.how.tap.b}</p>
            </div>
            <div className="step">
              <div className="n" />
              <h3>{c.how.auth.t}</h3>
              <p>{c.how.auth.b}</p>
            </div>
            <div className="step">
              <div className="n" />
              <h3>{c.how.settle.t}</h3>
              <p>{c.how.settle.b}</p>
            </div>
          </div>
        </div>
      </section>

      <section id="terminals" className="term-band">
        <div className="wrap">
          <div className="sec-head">
            <div className="eyebrow">{c.terminals.eyebrow}</div>
            <h2>{c.terminals.h2}</h2>
            <p>{c.terminals.lead}</p>
          </div>
          <div className="term">
            <div className="t">
              <div className="ic">
                <svg
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="6" y="2" width="12" height="20" rx="2.5" />
                  <path d="M11 18h2" />
                </svg>
              </div>
              <h3>{c.terminals.android.t}</h3>
              <p>{c.terminals.android.b}</p>
              <div className="t-foot">
                <span className="pill">{c.terminals.android.pill}</span>
                <a
                  className="dl"
                  href={DL_ANDROID}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {c.terminals.download} <span className="arw">→</span>
                </a>
              </div>
            </div>
            <div className="t">
              <div className="ic">
                <svg
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="3" y="4" width="18" height="12" rx="2" />
                  <path d="M8 20h8M12 16v4" />
                </svg>
              </div>
              <h3>{c.terminals.desktop.t}</h3>
              <p>{c.terminals.desktop.b}</p>
              <div className="t-foot">
                <span className="pill">{c.terminals.desktop.pill}</span>
                <a
                  className="dl"
                  href={DL_DESKTOP}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {c.terminals.download} <span className="arw">→</span>
                </a>
              </div>
            </div>
            <div className="t">
              <div className="ic">
                <svg
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M8 6l-5 6 5 6M16 6l5 6-5 6" />
                </svg>
              </div>
              <h3>{c.terminals.api.t}</h3>
              <p>{c.terminals.api.b}</p>
              <div className="t-foot">
                <span className="pill">{c.terminals.api.pill}</span>
                <a
                  className="dl"
                  href={GITHUB}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  GitHub <span className="arw">→</span>
                </a>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="cta-band">
        <div className="wrap">
          <h2>{c.cta.h2}</h2>
          <p>{c.cta.lead}</p>
          <div className="actions">
            <a className="btn btn-lineink" href={CONTACT}>
              {c.cta.contact} <span className="arw">→</span>
            </a>
            <Link className="btn btn-onink" href="/merchant">
              {c.cta.portal}
            </Link>
          </div>
        </div>
      </section>

      <footer>
        <div className="wrap">
          <div className="foot">
            <div className="col about">
              <a className="brand" href="#top">
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img
                  src="/melon-logo.png"
                  alt=""
                  style={{ width: 28, height: 28, borderRadius: 8 }}
                />{" "}
                Melon
              </a>
              <p>{c.footer.about}</p>
            </div>
            <div className="col">
              <h4>{c.footer.product}</h4>
              <a href="#features">{c.nav.features}</a>
              <a href="#how">{c.nav.how}</a>
              <a href="#terminals">{c.terminals.eyebrow}</a>
            </div>
            <div className="col">
              <h4>{c.footer.merchantHead}</h4>
              <Link href="/merchant">{c.footer.portal}</Link>
              <a href={CONTACT}>{c.footer.contact}</a>
            </div>
            <div className="col">
              <h4>{c.footer.resources}</h4>
              <a href={GITHUB} target="_blank" rel="noopener noreferrer">
                GitHub
              </a>
              <a href={DL_DESKTOP} target="_blank" rel="noopener noreferrer">
                {c.footer.dlDesktop}
              </a>
              <a href={DL_ANDROID} target="_blank" rel="noopener noreferrer">
                {c.footer.dlAndroid}
              </a>
            </div>
            <div className="col">
              <h4>{c.footer.legal}</h4>
              {/* Japanese only — the binding text. The English copy names the
                  language so the link does not promise a translation. */}
              <Link href="/terms" hrefLang="ja">
                {c.footer.terms}
              </Link>
              <Link href="/merchant-terms" hrefLang="ja">
                {c.footer.merchantTerms}
              </Link>
            </div>
          </div>
          <div className="legal">
            <span>{c.footer.note}</span>
            <span>© 2026 KIRISHIKI Yudai</span>
          </div>
        </div>
      </footer>
    </div>
  );
}
