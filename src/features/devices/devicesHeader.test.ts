import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const rootDir = path.resolve(import.meta.dirname, "../../..");

test("DevicesView header omits devices.title heading, removes leftHanded, and keeps truncating badge", () => {
  const src = fs.readFileSync(
    path.join(rootDir, "src/features/devices/DevicesView.tsx"),
    "utf8"
  );
  assert.doesNotMatch(src, /t\.devices\.title/);
  assert.doesNotMatch(src, /leftHanded/);
  assert.match(src, /flex-1 items-center min-w-0/);
  assert.match(src, /min-w-0 truncate text-xs/);
});

test("videolink is hidden and disabled on Android in App and SettingsView", () => {
  const appSrc = fs.readFileSync(path.join(rootDir, "src/App.tsx"), "utf8");
  const settingsSrc = fs.readFileSync(
    path.join(rootDir, "src/features/settings/SettingsView.tsx"),
    "utf8"
  );
  assert.match(appSrc, /videolinkEnabled\s*=\s*!isAndroid\s*&&\s*isModuleEnabled\("videolink"\)/);
  assert.match(settingsSrc, /isAndroid\s*&&\s*mod\.id\s*===\s*"videolink"/);
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
