import assert from "node:assert/strict";
import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import esbuild from "esbuild";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import type {
  LocalIdentityInfo,
  UpdateCheckResult,
  UpdateProgressEvent,
} from "../../core/types.ts";
import { translations } from "../../i18n/translations.ts";

const rootDir = path.resolve(import.meta.dirname, "../../..");
const require = createRequire(import.meta.url);

async function loadDevicesViewComponent() {
  const entryPath = path.join(rootDir, "src/features/devices/DevicesView.tsx");
  const externalPkgs = new Set([
    "react",
    "react/jsx-runtime",
    "react/jsx-dev-runtime",
    "react-dom",
    "react-dom/client",
    "lucide-react",
  ]);
  const result = await esbuild.build({
    entryPoints: [entryPath],
    bundle: true,
    write: false,
    format: "esm",
    platform: "node",
    jsx: "automatic",
    plugins: [
      {
        name: "resolve-node-externals-and-tauri-stub",
        setup(build) {
          build.onResolve({ filter: /^@tauri-apps\// }, (args) => ({
            path: args.path,
            namespace: "tauri-stub",
          }));
          build.onLoad({ filter: /.*/, namespace: "tauri-stub" }, () => ({
            contents: `
              export async function invoke(cmd, args) {
                if (typeof globalThis.__TAURI_INVOKE_MOCK__ === "function") {
                  return globalThis.__TAURI_INVOKE_MOCK__(cmd, args);
                }
                return null;
              }
              export async function listen() { return () => {}; }
              export function getCurrentWebview() {
                return { onDragDropEvent: async () => () => {} };
              }
              export async function open() { return null; }
            `,
            loader: "js",
          }));
          build.onResolve({ filter: /.*/ }, (args) => {
            if (externalPkgs.has(args.path)) {
              return {
                path: pathToFileURL(require.resolve(args.path)).href,
                external: true,
              };
            }
            return null;
          });
        },
      },
    ],
  });

  const code = result.outputFiles[0].text;
  const dataUrl = `data:text/javascript;base64,${Buffer.from(code, "utf8").toString("base64")}`;
  const mod = await import(dataUrl);
  return mod.DevicesView as React.ComponentType<{
    t: (typeof translations)["ru"];
    identity: LocalIdentityInfo | null;
    peers: [];
    friendRequests: [];
    leftHanded: boolean;
    updateInfo: UpdateCheckResult | null;
    updateProgress: UpdateProgressEvent | null;
    onUpdateInfoChange: (next: UpdateCheckResult | null) => void;
    onError: (msg: string) => void;
    onNotice?: (msg: string) => void;
  }>;
}

function setupDom() {
  const dom = new JSDOM("<!doctype html><html><body><div id='root'></div></body></html>", {
    url: "http://localhost/",
  });
  const prevWindow = globalThis.window;
  const prevDocument = globalThis.document;
  const prevNavigatorDesc = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  const prevHTMLElement = globalThis.HTMLElement;
  const prevActEnv = (globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT;

  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    value: dom.window.navigator,
    configurable: true,
    writable: true,
  });

  return {
    dom,
    cleanup() {
      Object.assign(globalThis, {
        window: prevWindow,
        document: prevDocument,
        HTMLElement: prevHTMLElement,
        IS_REACT_ACT_ENVIRONMENT: prevActEnv,
      });
      if (prevNavigatorDesc) {
        Object.defineProperty(globalThis, "navigator", prevNavigatorDesc);
      }
      dom.window.close();
    },
  };
}

const androidIdentity: LocalIdentityInfo = {
  peerId: "local-android",
  deviceName: "Xiaomi Phone",
  os: "android",
  category: "phone",
  publicKeyHex: "11".repeat(32),
  quicPort: 40406,
};

const sampleUpdateInfo: UpdateCheckResult = {
  currentVersion: "0.5.0",
  latestVersion: "0.5.1",
  hasUpdate: true,
  releaseTag: "v0.5.1",
  releaseNotes: "Automatic Android update",
  publishedAt: "2026-11-24T12:00:00Z",
  asset: {
    platform: "android",
    arch: "arm64-v8a",
    format: "apk",
    fileName: "landa-v0.5.1-android-arm64-v8a.apk",
    size: 12_345_678,
    sha256: "a".repeat(64),
    downloadUrl:
      "https://github.com/rslnmzhn/Landa/releases/download/v0.5.1/landa-v0.5.1-android-arm64-v8a.apk",
  },
};

test("DevicesView on Android runs unified download -> verify -> install_update flow without manual Downloads step", async () => {
  const DevicesView = await loadDevicesViewComponent();
  const { dom, cleanup } = setupDom();
  const calls: Array<{ cmd: string; args: unknown }> = [];
  const errors: string[] = [];
  const cachedApkPath = "/data/user/0/dev.landa.app/cache/updates/landa-v0.5.1-android-arm64-v8a.apk";

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: unknown
  ) => {
    calls.push({ cmd, args });
    if (cmd === "download_update") {
      return cachedApkPath;
    }
    if (cmd === "install_update") {
      return null;
    }
    return null;
  };

  const container = dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(
        React.createElement(DevicesView, {
          t: translations.ru,
          identity: androidIdentity,
          peers: [],
          friendRequests: [],
          leftHanded: false,
          updateInfo: sampleUpdateInfo,
          updateProgress: null,
          onUpdateInfoChange: () => {},
          onError: (msg) => errors.push(msg),
        })
      );
    });

    const openUpdateBtn = dom.window.document.querySelector(
      `button[aria-label="${translations.ru.update.tooltip}"]`
    ) as HTMLButtonElement | null;
    assert.ok(openUpdateBtn, "update button should be rendered in header");

    await act(async () => {
      openUpdateBtn.click();
    });

    const dialog = dom.window.document.querySelector('[role="dialog"]');
    assert.ok(dialog, "update modal dialog should be open");
    const buttons = Array.from(dialog.querySelectorAll("button"));
    const actionBtn = buttons.find(
      (b) => b.textContent?.trim() === translations.ru.update.downloadAndInstall
    );
    assert.ok(
      actionBtn,
      `expected unified '${translations.ru.update.downloadAndInstall}' button on Android`
    );

    await act(async () => {
      actionBtn.click();
    });

    assert.deepEqual(
      calls.map((c) => c.cmd),
      ["download_update", "install_update"]
    );
    assert.deepEqual(calls[1].args, { filePath: cachedApkPath });
    assert.deepEqual(errors, []);
  } finally {
    await act(async () => {
      root.unmount();
    });
    delete (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
    cleanup();
  }
});

test("DevicesView on Android shows permissionHint on INSTALL_PERMISSION_REQUIRED and retries install without re-downloading", async () => {
  const DevicesView = await loadDevicesViewComponent();
  const { dom, cleanup } = setupDom();
  const calls: Array<{ cmd: string; args: unknown }> = [];
  const errors: string[] = [];
  const cachedApkPath = "/data/user/0/dev.landa.app/cache/updates/landa-v0.5.1-android-arm64-v8a.apk";
  let installAttempts = 0;

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: unknown
  ) => {
    calls.push({ cmd, args });
    if (cmd === "download_update") {
      return cachedApkPath;
    }
    if (cmd === "install_update") {
      installAttempts += 1;
      if (installAttempts === 1) {
        throw new Error("INSTALL_PERMISSION_REQUIRED");
      }
      return null;
    }
    return null;
  };

  const container = dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(
        React.createElement(DevicesView, {
          t: translations.ru,
          identity: androidIdentity,
          peers: [],
          friendRequests: [],
          leftHanded: false,
          updateInfo: sampleUpdateInfo,
          updateProgress: null,
          onUpdateInfoChange: () => {},
          onError: (msg) => errors.push(msg),
        })
      );
    });

    const openUpdateBtn = dom.window.document.querySelector(
      `button[aria-label="${translations.ru.update.tooltip}"]`
    ) as HTMLButtonElement;
    await act(async () => {
      openUpdateBtn.click();
    });

    const dialog = dom.window.document.querySelector('[role="dialog"]');
    assert.ok(dialog, "update modal dialog should be open");
    const updateBtn = Array.from(dialog.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === translations.ru.update.downloadAndInstall
    );
    assert.ok(updateBtn, "expected Update button before first click");

    await act(async () => {
      updateBtn.click();
    });

    assert.ok(
      dialog.textContent?.includes(translations.ru.update.permissionHint),
      "expected permissionHint after INSTALL_PERMISSION_REQUIRED"
    );
    assert.deepEqual(errors, []);

    const installNowBtn = Array.from(dialog.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === translations.ru.update.installNow
    );
    assert.ok(installNowBtn, "expected Install button after permission prompt");

    await act(async () => {
      installNowBtn.click();
    });

    assert.deepEqual(
      calls.map((c) => c.cmd),
      ["download_update", "install_update", "install_update"]
    );
    assert.deepEqual(calls[2].args, { filePath: cachedApkPath });
    assert.deepEqual(errors, []);
  } finally {
    await act(async () => {
      root.unmount();
    });
    delete (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
    cleanup();
  }
});

test("AndroidManifest and CI workflow allow REQUEST_INSTALL_PACKAGES while forbidding storage permissions", () => {
  const manifest = fs.readFileSync(
    path.join(rootDir, "src-tauri/gen/android/app/src/main/AndroidManifest.xml"),
    "utf8"
  );
  assert.match(manifest, /android\.permission\.REQUEST_INSTALL_PACKAGES/);
  assert.match(manifest, /<queries>[\s\S]*application\/vnd\.android\.package-archive[\s\S]*<\/queries>/);
  assert.doesNotMatch(manifest, /MANAGE_EXTERNAL_STORAGE/);
  assert.doesNotMatch(manifest, /READ_EXTERNAL_STORAGE/);

  const workflow = fs.readFileSync(
    path.join(rootDir, ".github/workflows/build.yml"),
    "utf8"
  );
  assert.match(workflow, /for forbidden in MANAGE_EXTERNAL_STORAGE READ_EXTERNAL_STORAGE;/);
  assert.doesNotMatch(workflow, /for forbidden in[^\n]*REQUEST_INSTALL_PACKAGES/);
});
