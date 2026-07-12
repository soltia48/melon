import Link from "next/link";

const CONTACT = "https://unknowntech.jp/contact";
const GITHUB = "https://github.com/soltia48/melon";
const DL_DESKTOP = "https://github.com/soltia48/melon/releases";
const DL_ANDROID = "https://github.com/soltia48/MelonTerminal-Android/releases";

export default function Home() {
  return (
    <div className="lp">
      <nav className="nav">
        <div className="wrap">
          <a className="brand" href="#top">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src="/melon-logo.png" alt="" /> Melon
          </a>
          <div className="links">
            <a href="#features">特徴</a>
            <a href="#how">仕組み</a>
            <a href="#terminals">端末</a>
          </div>
          <div className="spacer" />
          <div className="cta-min">
            <a className="btn btn-ghost btn-sm" href={CONTACT}>
              お問い合わせ
            </a>
            <Link className="btn btn-primary btn-sm" href="/merchant">
              加盟店ポータル
            </Link>
          </div>
        </div>
      </nav>

      <header className="hero" id="top">
        <div className="wrap">
          <div>
            <div className="eyebrow anim d1">オンライン前払式支払手段プラットフォーム</div>
            <h1 className="anim d2">
              かざすだけ。
              <br />
              支払いも、チャージも、<span className="hl">一瞬</span>で。
            </h1>
            <p className="lead anim d3">
              カードをかざすだけで、支払い・チャージ・残高照会。利用者に専用アプリはいりません。
              残高はサーバ側の台帳で安全に管理し、相互認証で card-present を検証します。
            </p>
            <div className="actions anim d4">
              <Link className="btn btn-primary" href="/merchant">
                加盟店ポータルへ <span className="arw">→</span>
              </Link>
              <a className="btn btn-ghost" href={CONTACT}>
                導入のお問い合わせ
              </a>
            </div>
            <p className="microcopy anim d4">
              加盟店 API キーだけで、Android・デスクトップ端末・API から。
            </p>
          </div>

          <div className="card-hero anim d3" aria-hidden="true">
            <span className="ping" />
            <div className="badge">
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img src="/melon-logo.png" alt="Melon" />
            </div>
            <div className="tapline">カードをかざしてください</div>
            <div className="tapsub">FeliCa Standard · 相互認証で検証済み</div>
            <div className="chips">
              <div className="chip">
                <div className="k">利用可能残高</div>
                <div className="v">¥65,535</div>
              </div>
              <div className="chip">
                <div className="k">有効期限</div>
                <div className="v mono">2027-01-12</div>
              </div>
            </div>
          </div>
        </div>
      </header>

      <section className="values" aria-label="Melon の手軽さ">
        <div className="wrap">
          <div className="value">
            <div className="vic">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <path d="M7 8.5c2.4 1.8 2.4 5.2 0 7M11 5.5c4 3 4 9.5 0 12.5M15 3c5.4 4 5.4 13 0 17" />
              </svg>
            </div>
            <div>
              <h3>かざすだけの操作</h3>
              <p>支払いもチャージも、カードをかざすだけ。迷う操作はありません。</p>
            </div>
          </div>
          <div className="value">
            <div className="vic">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="6" width="18" height="12" rx="2" />
                <path d="M3 10h18M7 14h4" />
              </svg>
            </div>
            <div>
              <h3>利用者は専用アプリ不要</h3>
              <p>手持ちの FeliCa が、そのまま使えます。</p>
            </div>
          </div>
          <div className="value">
            <div className="vic">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="8" cy="8" r="4.2" />
                <path d="M11 11l7 7M15.5 17.5l2-2M13.5 19.5l2-2" />
              </svg>
            </div>
            <div>
              <h3>加盟店はキー1つで導入</h3>
              <p>API キーを入れるだけ。Android でもデスクトップでもすぐに。</p>
            </div>
          </div>
        </div>
      </section>

      <section id="features">
        <div className="wrap">
          <div className="sec-head">
            <div className="eyebrow">なぜ Melon か</div>
            <h2>タップの正しさを、仕組みで担保する。</h2>
            <p>
              信頼の起点はカードの主張ではなく、サーバ側のオンライン相互認証。その上に、消せない台帳と規制に配慮した失効を重ねています。
            </p>
          </div>
          <div className="grid">
            <article className="feature">
              <div className="ic">
                <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M12 3l7 3v5c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z" />
                  <path d="M9.2 12l2 2 3.6-3.8" />
                </svg>
              </div>
              <h3>サーバ検証済みの本人性</h3>
              <p>
                オンライン相互認証で得た IDi をアカウントキーに。加盟店は暗号鍵を持たず、残高もカードに書かないため、なりすましも改ざんもできません。
              </p>
            </article>
            <article className="feature">
              <div className="ic">
                <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M6 3h9l4 4v14H6z" />
                  <path d="M9 8h5M9 12h7M9 16h7" />
                </svg>
              </div>
              <h3>追記専用の不変台帳</h3>
              <p>
                すべての入出金を消せない台帳に記録。冪等キーで二重支払いを防ぎ、返金は元のチャージへ正しく戻します。残高はいつでも台帳から再構築可能。
              </p>
            </article>
            <article className="feature">
              <div className="ic">
                <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="12" cy="13" r="8" />
                  <path d="M12 9v4l2.5 2M9 2h6" />
                </svg>
              </div>
              <h3>6 か月失効(資金決済法に配慮)</h3>
              <p>
                チャージごとに JST で 6 か月失効。発行日から 6 か月以内の適用除外を、厳密・監査可能・不変に実装しています。
              </p>
            </article>
            <article className="feature">
              <div className="ic">
                <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                  <rect x="3" y="4" width="18" height="16" rx="2" />
                  <path d="M3 9h18M9 9v11" />
                </svg>
              </div>
              <h3>取引・返金・精算を一元管理</h3>
              <p>
                残高・取引・返金・精算・スタッフ管理まで。加盟店は専用ポータルから、日々の運用に必要な操作だけを役割ごとに行えます。
              </p>
            </article>
          </div>
        </div>
      </section>

      <section id="how" className="alt">
        <div className="wrap">
          <div className="sec-head">
            <div className="eyebrow">仕組み</div>
            <h2>タップが、決済になるまで。</h2>
            <p>
              端末はカードとサーバのフレームを中継するだけ。暗号鍵はサーバが握り、決済はサーバの台帳で確定します。
            </p>
          </div>
          <div className="steps">
            <div className="step">
              <div className="n" />
              <h3>カードをかざす</h3>
              <p>加盟店端末が FeliCa をポーリングし、IDm / PMm を取得。端末は鍵を持ちません。</p>
            </div>
            <div className="step">
              <div className="n" />
              <h3>サーバで相互認証</h3>
              <p>端末はフレームを中継するだけ。サーバが鍵で認証し、検証済み IDi と認証済みセッションを得ます。</p>
            </div>
            <div className="step">
              <div className="n" />
              <h3>残高で決済</h3>
              <p>そのセッションで支払い・チャージ・残高照会・返金。台帳に記帳して完了します。</p>
            </div>
          </div>
        </div>
      </section>

      <section id="terminals" className="term-band">
        <div className="wrap">
          <div className="sec-head">
            <div className="eyebrow">対応端末</div>
            <h2>どの端末からでも、同じ台帳へ。</h2>
            <p>加盟店 API キーで認証。金銭操作は冪等キーで二重処理を防ぎます。</p>
          </div>
          <div className="term">
            <div className="t">
              <div className="ic">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                  <rect x="6" y="2" width="12" height="20" rx="2.5" />
                  <path d="M11 18h2" />
                </svg>
              </div>
              <h3>Android アプリ</h3>
              <p>NFC 対応端末を、かざすだけの POS 端末に。テンキー入力と確認ボタンで誤タップも防止。</p>
              <div className="t-foot">
                <span className="pill">NFC-F / FeliCa</span>
                <a className="dl" href={DL_ANDROID} target="_blank" rel="noopener noreferrer">
                  ダウンロード <span className="arw">→</span>
                </a>
              </div>
            </div>
            <div className="t">
              <div className="ic">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                  <rect x="3" y="4" width="18" height="12" rx="2" />
                  <path d="M8 20h8M12 16v4" />
                </svg>
              </div>
              <h3>デスクトップ端末</h3>
              <p>PaSoRi を接続した常設レジ向け。ブラウザの Web UI キオスクとして起動します。</p>
              <div className="t-foot">
                <span className="pill">Win / macOS / Linux</span>
                <a className="dl" href={DL_DESKTOP} target="_blank" rel="noopener noreferrer">
                  ダウンロード <span className="arw">→</span>
                </a>
              </div>
            </div>
            <div className="t">
              <div className="ic">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M8 6l-5 6 5 6M16 6l5 6-5 6" />
                </svg>
              </div>
              <h3>REST API</h3>
              <p>相互認証・決済・返金・残高照会を JSON で。独自端末や基幹システムへ直接統合。</p>
              <div className="t-foot">
                <span className="pill">/v1</span>
                <a className="dl" href={GITHUB} target="_blank" rel="noopener noreferrer">
                  GitHub <span className="arw">→</span>
                </a>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="cta-band">
        <div className="wrap">
          <h2>導入をご検討ですか?</h2>
          <p>
            Melon の導入・お見積りは、お問い合わせページからご相談ください。すでに加盟店の方は、ポータルからサインインできます。
          </p>
          <div className="actions">
            <a className="btn btn-lineink" href={CONTACT}>
              お問い合わせ <span className="arw">→</span>
            </a>
            <Link className="btn btn-onink" href="/merchant">
              加盟店ポータル
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
                <img src="/melon-logo.png" alt="" style={{ width: 28, height: 28, borderRadius: 8 }} /> Melon
              </a>
              <p>FeliCa IDi ベースのオンライン前払式支払手段プラットフォーム。</p>
            </div>
            <div className="col">
              <h4>プロダクト</h4>
              <a href="#features">特徴</a>
              <a href="#how">仕組み</a>
              <a href="#terminals">対応端末</a>
            </div>
            <div className="col">
              <h4>加盟店</h4>
              <Link href="/merchant">加盟店ポータル</Link>
              <a href={CONTACT}>お問い合わせ</a>
            </div>
            <div className="col">
              <h4>リソース</h4>
              <a href={GITHUB} target="_blank" rel="noopener noreferrer">
                GitHub
              </a>
              <a href={DL_DESKTOP} target="_blank" rel="noopener noreferrer">
                デスクトップ版ダウンロード
              </a>
              <a href={DL_ANDROID} target="_blank" rel="noopener noreferrer">
                Android 版ダウンロード
              </a>
            </div>
          </div>
          <div className="legal">
            <span>
              Melon は前払式支払手段(第三者型)の発行・管理基盤です。資金決済法に基づく表示等は発行者が別途行います。
            </span>
            <span>© 2026 KIRISHIKI Yudai</span>
          </div>
        </div>
      </footer>
    </div>
  );
}
