import fs from "node:fs";
import path from "node:path";
import { ImageResponse } from "next/og";
import { SITE_HOST } from "@/lib/site";

/**
 * Open Graph card images (/opengraph-image, /terms/opengraph-image, …).
 *
 * Rendered by Satori at BUILD time, so these are plain static PNGs at runtime.
 * Satori has no access to system fonts and its built-in font has no Japanese
 * glyphs — every glyph we draw must come from a font we hand it, or it renders
 * as blank boxes. `assets/NotoSansJP-*.woff` are Noto Sans CJK JP subset to just
 * the characters used below (see assets/README.md); re-subset if you add copy
 * with new kanji.
 */
export const OG_SIZE = { width: 1200, height: 630 };
export const OG_CONTENT_TYPE = "image/png";

// Read lazily, never at module scope: Next loads this route's module on boot even
// when it serves the prerendered PNG, so a top-level read would crash the server
// at startup rather than at render time.
const read = (...p: string[]) =>
  fs.readFileSync(path.join(process.cwd(), ...p));

const logo = () =>
  `data:image/png;base64,${read("public", "melon-logo.png").toString("base64")}`;

const fonts = () => [
  {
    name: "Noto Sans JP",
    data: read("assets", "NotoSansJP-Regular.woff"),
    weight: 400 as const,
    style: "normal" as const,
  },
  {
    name: "Noto Sans JP",
    data: read("assets", "NotoSansJP-Bold.woff"),
    weight: 700 as const,
    style: "normal" as const,
  },
];

const BG = "#0c1410";
const TEXT = "#f2f5f3";
const ACCENT = "#37c26f";
const MUTED = "#9bb3a4";

/** One card: brand lockup, headline lines, and a subtitle. */
export function ogCard(lines: string[], subtitle: string) {
  const LOGO = logo();
  return new ImageResponse(
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        padding: "64px 72px",
        background: BG,
        // Linear, not radial: the renderer behind next/og draws radial
        // gradients in visible rings.
        backgroundImage:
          "linear-gradient(115deg, #0c1410 0%, #0f2118 55%, #143327 100%)",
        color: TEXT,
        fontFamily: "Noto Sans JP",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 20 }}>
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={LOGO}
          width={80}
          height={80}
          alt=""
          style={{ borderRadius: 20 }}
        />
        <span
          style={{ fontSize: 44, fontWeight: 700, letterSpacing: "-0.01em" }}
        >
          Melon
        </span>
      </div>

      <div style={{ display: "flex", flexDirection: "column" }}>
        {lines.map((line) => (
          <span
            key={line}
            style={{
              fontSize: 68,
              fontWeight: 700,
              lineHeight: 1.28,
              letterSpacing: "-0.02em",
            }}
          >
            {line}
          </span>
        ))}
        <span
          style={{
            marginTop: 24,
            fontSize: 30,
            fontWeight: 400,
            color: ACCENT,
          }}
        >
          {subtitle}
        </span>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
        <div
          style={{ width: 56, height: 4, background: ACCENT, borderRadius: 2 }}
        />
        <span style={{ fontSize: 26, color: MUTED }}>{SITE_HOST}</span>
      </div>
    </div>,
    { ...OG_SIZE, fonts: fonts() },
  );
}
