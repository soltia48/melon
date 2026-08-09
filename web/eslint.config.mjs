// ESLint flat config.
//
// Next 16 removed the `next lint` subcommand, so the ESLint CLI is invoked
// directly (`npm run lint`). `eslint-config-next` ships native flat configs, so
// no eslintrc compatibility layer is needed.
//
//   core-web-vitals — the Next rules plus the stricter Core Web Vitals set
//   typescript      — the TypeScript parser and its recommended rules

import nextCoreWebVitals from "eslint-config-next/core-web-vitals";
import nextTypeScript from "eslint-config-next/typescript";

const config = [
  // Build output, dependencies and the generated Next ambient types. `.next` in
  // particular holds the standalone bundle, which is a full copy of the source.
  {
    ignores: [".next/**", "node_modules/**", "next-env.d.ts"],
  },
  ...nextCoreWebVitals,
  ...nextTypeScript,
];

export default config;
