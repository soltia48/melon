import Link from "next/link";

export default function Home() {
  return (
    <div className="landing">
      <div style={{ textAlign: "center" }}>
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src="/melon-logo.png"
          alt="Melon"
          width={80}
          height={80}
          style={{ display: "block", margin: "0 auto 10px", borderRadius: 18 }}
        />
        <h1 style={{ fontSize: 26 }}>Melon</h1>
        <p className="muted">オンライン前払式支払手段のコンソール</p>
        <div className="choices">
          <Link href="/admin" className="choice">
            <div className="t">管理画面(発行者)</div>
            <div className="muted">加盟店・利用者・取引・発行者残高の管理</div>
          </Link>
          <Link href="/merchant" className="choice">
            <div className="t">加盟店ポータル</div>
            <div className="muted">取引・返金・スタッフの管理</div>
          </Link>
        </div>
      </div>
    </div>
  );
}
