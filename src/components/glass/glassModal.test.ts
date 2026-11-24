import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const rootDir = path.resolve(import.meta.dirname, "../../..");

test("animate-glass-view does not retain transform or filter containing block after animation", () => {
  const css = fs.readFileSync(path.join(rootDir, "src/styles/glass.css"), "utf8");

  const viewRuleMatch = css.match(/\.animate-glass-view\s*\{([^}]*)\}/);
  assert.ok(viewRuleMatch, ".animate-glass-view rule must exist");
  assert.doesNotMatch(
    viewRuleMatch[1],
    /\b(both|forwards)\b/,
    ".animate-glass-view must not use animation-fill-mode both/forwards"
  );

  const keyframeMatch = css.match(/@keyframes\s+glassViewEnter\s*\{([\s\S]*?)\n\}/);
  assert.ok(keyframeMatch, "glassViewEnter keyframes must exist");
  assert.match(keyframeMatch[1], /100%\s*\{[\s\S]*transform:\s*none;/);
  assert.match(keyframeMatch[1], /100%\s*\{[\s\S]*filter:\s*none;/);
});

test("liquid-glass-modal defines high-contrast opaque surface tokens", () => {
  const css = fs.readFileSync(path.join(rootDir, "src/styles/glass.css"), "utf8");
  assert.match(css, /--glass-bg-modal:\s*rgba\(/);
  assert.match(css, /\.liquid-glass-modal\s*\{[\s\S]*background:\s*var\(--glass-bg-modal\)/);
});

test("GlassModal renders via createPortal into document.body with dialog semantics and dense backdrop", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/components/glass/GlassPrimitives.tsx"),
    "utf8"
  );
  assert.match(src, /import\s*\{\s*createPortal\s*\}\s*from\s*"react-dom"/);
  assert.match(src, /createPortal\(modal,\s*document\.body\)/);
  assert.match(src, /role="dialog"/);
  assert.match(src, /aria-modal="true"/);
  assert.match(src, /liquid-glass-modal/);
  assert.match(src, /text-slate-900\/90\s+dark:text-white\/90/);
  assert.match(src, /bg-slate-950\/65\s+dark:bg-black\/75/);
});
