import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { resolveTabSwipe } from "../../core/tabSwipe.ts";

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
  assert.doesNotMatch(keyframeMatch[1], /filter:/);
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

test("root scrollbars are visually hidden to prevent layout shift when expanding lists", () => {
  const css = fs.readFileSync(path.join(rootDir, "src/styles/glass.css"), "utf8");
  assert.match(css, /scrollbar-width:\s*none;/);
  assert.match(css, /::-webkit-scrollbar[\s\S]*display:\s*none;/);
});

test("SettingsView exposes download folder picker button on Android via pickDownloadDirAndroid", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/features/settings/SettingsView.tsx"),
    "utf8"
  );
  assert.match(src, /api\.pickDownloadDirAndroid\(\)/);
  assert.doesNotMatch(src, /\{!isAndroid\s*&&\s*\(\s*<GlassButton[\s\S]*handlePickDownloadDir/);
});

test("MeshCanvas uses static radial gradients without heavy blur or infinite float animations", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/components/glass/GlassPrimitives.tsx"),
    "utf8"
  );
  assert.doesNotMatch(src, /animate-float-/);
  assert.doesNotMatch(src, /blur-\[1[0-9]{2}px\]/);
  assert.match(src, /radial-gradient\(/);
});

test("resolveTabSwipe switches tabs on horizontal swipe and ignores vertical scroll", () => {
  const tabs = ["devices", "shares", "clipboard", "activity", "settings"];
  assert.equal(resolveTabSwipe(tabs, "devices", -80, 10, 200), "shares");
  assert.equal(resolveTabSwipe(tabs, "shares", 80, -12, 200), "devices");
  assert.equal(resolveTabSwipe(tabs, "devices", 80, 0, 200), null);
  assert.equal(resolveTabSwipe(tabs, "settings", -80, 0, 200), null);
  assert.equal(resolveTabSwipe(tabs, "shares", -40, 60, 200), null);
  assert.equal(resolveTabSwipe(tabs, "shares", -42, 5, 80), "clipboard");
});

test("moving lens indicator uses CSS liquid-lens-track and respects prefers-reduced-motion", () => {
  const css = fs.readFileSync(path.join(rootDir, "src/styles/glass.css"), "utf8");
  const primitives = fs.readFileSync(
    path.join(rootDir, "src/components/glass/GlassPrimitives.tsx"),
    "utf8"
  );
  assert.match(primitives, /liquid-lens-track/);
  assert.doesNotMatch(primitives, /transition:\s*"transform 320ms/);
  assert.match(
    css,
    /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*\.liquid-lens-track[\s\S]*transition:\s*none;/
  );
});
