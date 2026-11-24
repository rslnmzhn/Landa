import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const rootDir = path.resolve(import.meta.dirname, "../../..");

test("api exposes forgetPeer invoking forget_peer command", () => {
  const src = fs.readFileSync(path.join(rootDir, "src/core/api.ts"), "utf8");
  assert.match(src, /forgetPeer:\s*\(peerId:\s*string\)\s*=>\s*invoke<void>\("forget_peer",\s*\{\s*peerId\s*\}\)/);
});

test("DevicesView provides forget device action with confirmation modal", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/features/devices/DevicesView.tsx"),
    "utf8"
  );
  assert.match(src, /forgetTarget/);
  assert.match(src, /api\.forgetPeer\(/);
  assert.match(src, /t\.devices\.forget/);
  assert.match(src, /t\.devices\.forgetConfirmTitle/);
  assert.match(src, /t\.devices\.forgetConfirmDesc/);
  assert.match(src, /t\.devices\.forgetConfirmAction/);
});

test("translations define RU and EN strings for forgetting a device", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/i18n/translations.ts"),
    "utf8"
  );
  assert.match(src, /forget:\s*"Забыть устройство"/);
  assert.match(src, /forget:\s*"Forget device"/);
  assert.match(src, /forgetConfirmTitle:/);
  assert.match(src, /forgetConfirmDesc:/);
  assert.match(src, /forgetConfirmAction:/);
});
