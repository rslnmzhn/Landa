import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const rootDir = path.resolve(import.meta.dirname, "../../..");

test("DevicesView header omits devices.title heading and keeps truncating badge with leftHanded support", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/features/devices/DevicesView.tsx"),
    "utf8"
  );
  assert.doesNotMatch(src, /t\.devices\.title/);
  assert.match(src, /flex-1 items-center min-w-0/);
  assert.match(src, /min-w-0 truncate text-xs/);
  assert.match(src, /leftHanded \? "flex-row-reverse" : ""/);
  assert.match(src, /leftHanded \? "justify-end" : ""/);
});

test("GlassBadge constrains width so long device names truncate instead of overlapping buttons", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/components/glass/GlassPrimitives.tsx"),
    "utf8"
  );
  assert.match(src, /inline-flex min-w-0 max-w-full items-center/);
});

test("App syncs identity.deviceName immediately when settings are updated", () => {
  const src = fs.readFileSync(path.join(rootDir, "src/App.tsx"), "utf8");
  assert.match(src, /deviceName:\s*next\.deviceName/);
});
