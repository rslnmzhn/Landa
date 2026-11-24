import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "../../..");

test("Android network_security_config permits cleartext strictly for 127.0.0.1 and localhost", () => {
  const configPath = resolve(
    root,
    "src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml"
  );
  assert.equal(existsSync(configPath), true, "network_security_config.xml must exist");

  const xml = readFileSync(configPath, "utf8");
  assert.match(xml, /<base-config\s+cleartextTrafficPermitted="false"\s*\/?>/);
  assert.match(xml, /<domain-config\s+cleartextTrafficPermitted="true">/);
  assert.match(xml, /<domain\s+includeSubdomains="false">127\.0\.0\.1<\/domain>/);
  assert.match(xml, /<domain\s+includeSubdomains="false">localhost<\/domain>/);

  const manifest = readFileSync(
    resolve(root, "src-tauri/gen/android/app/src/main/AndroidManifest.xml"),
    "utf8"
  );
  assert.match(
    manifest,
    /android:networkSecurityConfig="@xml\/network_security_config"/
  );
});

test("ActivityView resolves content:// display names and uses openFileExternal on Android", () => {
  const activityView = readFileSync(
    resolve(root, "src/features/activity/ActivityView.tsx"),
    "utf8"
  );
  assert.match(activityView, /resolvePreviewNames/);
  assert.match(activityView, /isAndroid/);
  assert.match(activityView, /api\.openFileExternal/);
});
