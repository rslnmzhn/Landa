import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import path, { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import esbuild from "esbuild";
import { JSDOM } from "jsdom";
import React, { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import type {
  TransferHistoryRecord,
  TransferProgressInfo,
} from "../../core/types.ts";
import { translations } from "../../i18n/translations.ts";
import {
  extractFileName,
  summarizeTransferPaths,
} from "./activityPaths.ts";

const root = resolve(import.meta.dirname, "../../..");
const require = createRequire(import.meta.url);

async function loadActivityViewComponent() {
  const entryPath = path.join(root, "src/features/activity/ActivityView.tsx");
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
                return {};
              }
              export async function listen() { return () => {}; }
              export async function openPath() {}
              export async function revealItemInDir() {}
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
  return mod.ActivityView as React.ComponentType<{
    t: (typeof translations)["ru"];
    activeTransfers: TransferProgressInfo[];
    history: TransferHistoryRecord[];
    onPreviewPath: (pathOrUri: string) => void;
    onError: (msg: string) => void;
  }>;
}

function setupDomEnvironment() {
  const dom = new JSDOM(
    '<!doctype html><html><body><div id="root"></div></body></html>',
    { url: "http://localhost/" }
  );
  const win = dom.window;

  const prevGlobals = {
    window: globalThis.window,
    document: globalThis.document,
    HTMLElement: globalThis.HTMLElement,
    Node: globalThis.Node,
    IS_REACT_ACT_ENVIRONMENT: (globalThis as Record<string, unknown>)
      .IS_REACT_ACT_ENVIRONMENT,
  };

  Object.assign(win, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    HTMLElement: win.HTMLElement,
    Node: win.Node,
    IS_REACT_ACT_ENVIRONMENT: true,
  });

  return {
    dom,
    cleanup() {
      Object.assign(globalThis, prevGlobals);
      dom.window.close();
    },
  };
}

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

test("ActivityView omits Eye icon for single received files and expanded multi-file items while keeping click-to-preview", async (t) => {
  const ActivityView = await loadActivityViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    env.cleanup();
  });

  const previewedPaths: string[] = [];

  await act(async () => {
    root.render(
      React.createElement(ActivityView, {
        t: translations.ru,
        activeTransfers: [],
        history: [
          {
            id: "single-1",
            peerId: "peer-1",
            peerName: "SinglePeer",
            direction: "receive",
            fileCount: 1,
            totalBytes: 1024,
            status: "completed",
            savedPaths: ["/downloads/photo.png"],
            errorMessage: null,
            createdAt: 1000,
          },
          {
            id: "multi-1",
            peerId: "peer-2",
            peerName: "MultiPeer",
            direction: "receive",
            fileCount: 2,
            totalBytes: 4096,
            status: "completed",
            savedPaths: ["/downloads/doc1.txt", "/downloads/doc2.txt"],
            errorMessage: null,
            createdAt: 2000,
          },
        ],
        onPreviewPath: (p) => previewedPaths.push(p),
        onError: () => {},
      })
    );
  });

  assert.equal(
    container.querySelectorAll("svg.lucide-eye").length,
    0,
    "single received file row must not render Eye icon"
  );

  const findButtonByText = (text: string) =>
    Array.from(container.querySelectorAll("button")).find((btn) =>
      btn.textContent?.includes(text)
    );

  const singleRowBtn = findButtonByText("SinglePeer");
  assert.ok(singleRowBtn, "single received file row button should render");
  await act(async () => {
    singleRowBtn.click();
  });
  assert.deepEqual(previewedPaths, ["/downloads/photo.png"]);

  const multiRowBtn = findButtonByText("MultiPeer");
  assert.ok(multiRowBtn, "multi-file row button should render");
  await act(async () => {
    multiRowBtn.click();
  });

  assert.equal(
    container.querySelectorAll("svg.lucide-eye").length,
    0,
    "expanded multi-file list must not render Eye icons"
  );

  const doc2Btn = findButtonByText("doc2.txt");
  assert.ok(doc2Btn, "expanded file item button should render");
  await act(async () => {
    doc2Btn.click();
  });
  assert.deepEqual(previewedPaths, [
    "/downloads/photo.png",
    "/downloads/doc2.txt",
  ]);
});

test("ActivityView displays file names, relative subfolder paths, and save paths for received and sent transfers", async (t) => {
  const ActivityView = await loadActivityViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "resolve_preview_names") {
      const paths = (args.paths as string[]) ?? [];
      const out: Record<string, string> = {};
      for (const p of paths) {
        if (p === "content://media/external/downloads/101") {
          out[p] = "Downloads/Landa/vacation.jpg";
        } else if (p === "content://media/external/downloads/201") {
          out[p] = "Downloads/Landa/Trip/day1/beach.png";
        } else if (p === "content://media/external/downloads/202") {
          out[p] = "Downloads/Landa/Trip/notes.txt";
        }
      }
      return out;
    }
    return {};
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  await act(async () => {
    root.render(
      React.createElement(ActivityView, {
        t: translations.ru,
        activeTransfers: [],
        history: [
          {
            id: "recv-single-android",
            peerId: "peer-1",
            peerName: "PhonePeer",
            direction: "receive",
            fileCount: 1,
            totalBytes: 2048,
            status: "completed",
            savedPaths: ["content://media/external/downloads/101"],
            errorMessage: null,
            createdAt: 1000,
          },
          {
            id: "send-single-desktop",
            peerId: "peer-2",
            peerName: "LaptopPeer",
            direction: "send",
            fileCount: 1,
            totalBytes: 4096,
            status: "completed",
            savedPaths: ["C:\\Users\\alice\\Documents\\report.pdf"],
            errorMessage: null,
            createdAt: 2000,
          },
          {
            id: "recv-folder-android",
            peerId: "peer-3",
            peerName: "FolderPeer",
            direction: "receive",
            fileCount: 2,
            totalBytes: 8192,
            status: "completed",
            savedPaths: [
              "content://media/external/downloads/201",
              "content://media/external/downloads/202",
            ],
            errorMessage: null,
            createdAt: 3000,
          },
          {
            id: "send-folder-desktop",
            peerId: "peer-4",
            peerName: "SentFolderPeer",
            direction: "send",
            fileCount: 2,
            totalBytes: 1024,
            status: "completed",
            savedPaths: [
              "/home/alice/Projects/demo/src/main.rs",
              "/home/alice/Projects/demo/README.md",
            ],
            errorMessage: null,
            createdAt: 4000,
          },
        ],
        onPreviewPath: () => {},
        onError: () => {},
      })
    );
  });

  // Single received file (resolved from content://) shows filename and Downloads/Landa/... path
  assert.ok(
    container.textContent?.includes("vacation.jpg"),
    "single received file must display resolved filename"
  );
  assert.ok(
    container.textContent?.includes("Downloads/Landa/vacation.jpg"),
    "single received file must display Downloads/Landa/... save path"
  );

  // Single sent file shows filename and source path
  assert.ok(
    container.textContent?.includes("report.pdf"),
    "single sent file must display filename"
  );
  assert.ok(
    container.textContent?.includes("C:\\Users\\alice\\Documents\\report.pdf"),
    "single sent file must display source path"
  );

  // Multi-file received folder shows common destination path and expands to show relative subfolder paths + full save paths
  assert.ok(
    container.textContent?.includes("Downloads/Landa/Trip"),
    "multi-file received folder card must display common destination path"
  );
  const folderRowBtn = Array.from(container.querySelectorAll("button")).find(
    (btn) => btn.textContent?.includes("FolderPeer")
  );
  assert.ok(folderRowBtn);
  await act(async () => {
    folderRowBtn.click();
  });
  assert.ok(
    container.textContent?.includes("day1/beach.png"),
    "expanded folder transfer must show relative subfolder path"
  );
  assert.ok(
    container.textContent?.includes("Downloads/Landa/Trip/day1/beach.png"),
    "expanded folder transfer must show full Downloads/Landa/... save path"
  );

  // Multi-file sent folder can also be expanded to show relative subfolder paths and source paths
  const sentFolderBtn = Array.from(container.querySelectorAll("button")).find(
    (btn) => btn.textContent?.includes("SentFolderPeer")
  );
  assert.ok(sentFolderBtn, "multi-file sent transfer must be expandable");
  await act(async () => {
    sentFolderBtn.click();
  });
  assert.ok(
    container.textContent?.includes("src/main.rs"),
    "expanded sent folder must show relative subfolder path"
  );
  assert.ok(
    container.textContent?.includes("/home/alice/Projects/demo/src/main.rs"),
    "expanded sent folder must show full source path"
  );
});

test("activityPaths handles trailing separators, SAF URIs, mixed separators, and partial resolution", () => {
  assert.equal(extractFileName("/home/alice/folder/"), "folder");
  assert.equal(extractFileName("C:\\Users\\alice\\docs\\"), "docs");
  assert.equal(
    extractFileName(
      "content://com.android.externalstorage.documents/document/primary%3ADownload%2FLanda%2Fvacation.jpg?foo=1"
    ),
    "vacation.jpg"
  );
  assert.equal(extractFileName("/tmp/what?%20file.txt"), "what?%20file.txt");

  const mixedWin = summarizeTransferPaths(
    [
      "C:\\Users\\alice\\Downloads/Landa/Trip/a.txt",
      "C:\\Users\\alice\\Downloads\\Landa\\Trip\\sub\\b.txt",
    ],
    {}
  );
  assert.equal(mixedWin.commonDir, "C:\\Users\\alice\\Downloads\\Landa\\Trip");
  assert.deepEqual(
    mixedWin.items.map((i) => i.relativePath),
    ["a.txt", "sub/b.txt"]
  );

  const partial = summarizeTransferPaths(
    [
      "content://media/external/downloads/1",
      "content://media/external/downloads/2",
      "content://media/external/downloads/3",
    ],
    {
      "content://media/external/downloads/1": "Downloads/Landa/Trip/day1/a.jpg",
      "content://media/external/downloads/2": "Downloads/Landa/Trip/b.jpg",
    }
  );
  assert.equal(partial.commonDir, "Downloads/Landa/Trip");
  assert.deepEqual(
    partial.items.map((i) => i.relativePath),
    ["day1/a.jpg", "b.jpg", "3"]
  );

  const rootLevel = summarizeTransferPaths(["/a.txt", "/b.txt"], {});
  assert.equal(rootLevel.commonDir, null);
  assert.deepEqual(
    rootLevel.items.map((i) => i.relativePath),
    ["a.txt", "b.txt"]
  );

  const nameOnlySingle = summarizeTransferPaths(
    ["content://provider/document/42"],
    { "content://provider/document/42": "report.pdf" }
  );
  assert.equal(nameOnlySingle.items[0].fileName, "report.pdf");
  assert.equal(
    nameOnlySingle.items[0].displayPath,
    "content://provider/document/42"
  );

  const fullPathSingle = summarizeTransferPaths(
    ["content://media/external/downloads/101"],
    { "content://media/external/downloads/101": "Downloads/Landa/report.pdf" }
  );
  assert.equal(fullPathSingle.items[0].fileName, "report.pdf");
  assert.equal(
    fullPathSingle.items[0].displayPath,
    "Downloads/Landa/report.pdf"
  );
});

test("ActivityView resolves content:// URIs under React.StrictMode and concurrent history updates without duplicating bare filenames", async (t) => {
  const ActivityView = await loadActivityViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  let resolveFirstBatch: ((val: Record<string, string>) => void) | null = null;
  let resolveSecondBatch: ((val: Record<string, string>) => void) | null = null;
  let callCount = 0;

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "resolve_preview_names") {
      callCount++;
      const paths = (args.paths as string[]) ?? [];
      if (paths.includes("content://media/external/downloads/501")) {
        return new Promise<Record<string, string>>((resolve) => {
          resolveFirstBatch = resolve;
        });
      }
      if (paths.includes("content://media/external/downloads/502")) {
        return new Promise<Record<string, string>>((resolve) => {
          resolveSecondBatch = resolve;
        });
      }
    }
    return Promise.resolve({});
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  const item1: TransferHistoryRecord = {
    id: "strict-1",
    peerId: "peer-1",
    peerName: "StrictPeer",
    direction: "receive",
    fileCount: 1,
    totalBytes: 512,
    status: "completed",
    savedPaths: ["content://media/external/downloads/501"],
    errorMessage: null,
    createdAt: 1000,
  };
  const item2: TransferHistoryRecord = {
    id: "strict-2",
    peerId: "peer-2",
    peerName: "BareNamePeer",
    direction: "receive",
    fileCount: 1,
    totalBytes: 256,
    status: "completed",
    savedPaths: ["content://media/external/downloads/502"],
    errorMessage: null,
    createdAt: 2000,
  };

  await act(async () => {
    root.render(
      React.createElement(
        StrictMode,
        null,
        React.createElement(ActivityView, {
          t: translations.ru,
          activeTransfers: [],
          history: [item1],
          onPreviewPath: () => {},
          onError: () => {},
        })
      )
    );
  });

  // StrictMode double-invocation must not fire duplicate IPC calls for the same in-flight URI
  assert.equal(callCount, 1);

  // Update history while first batch is still in flight
  await act(async () => {
    root.render(
      React.createElement(
        StrictMode,
        null,
        React.createElement(ActivityView, {
          t: translations.ru,
          activeTransfers: [],
          history: [item1, item2],
          onPreviewPath: () => {},
          onError: () => {},
        })
      )
    );
  });
  assert.equal(callCount, 2);

  await act(async () => {
    resolveFirstBatch?.({
      "content://media/external/downloads/501": "Downloads/Landa/strict.png",
    });
    resolveSecondBatch?.({
      "content://media/external/downloads/502": "bare-only.txt",
    });
  });

  assert.ok(
    container.textContent?.includes("Downloads/Landa/strict.png"),
    "StrictMode must not discard resolved Downloads/Landa/... path"
  );
  const bareMatches = container.textContent?.match(/bare-only\.txt/g) ?? [];
  assert.equal(
    bareMatches.length,
    1,
    "single-file card must not duplicate filename when provider returns only DISPLAY_NAME"
  );
  assert.ok(
    container.textContent?.includes("content://media/external/downloads/502"),
    "single-file card must show original content:// URI when provider returns only DISPLAY_NAME"
  );
});

test("ActivityView displays both filename and original content:// URI when provider returns only DISPLAY_NAME in single and expanded multi-file records", async (t) => {
  const ActivityView = await loadActivityViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "resolve_preview_names") {
      const paths = (args.paths as string[]) ?? [];
      const out: Record<string, string> = {};
      for (const p of paths) {
        if (p === "content://provider/document/42") {
          out[p] = "report.pdf";
        } else if (p === "content://provider/document/43") {
          out[p] = "notes.txt";
        } else if (p === "content://provider/document/44") {
          out[p] = "report.pdf";
        }
      }
      return out;
    }
    return {};
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  await act(async () => {
    root.render(
      React.createElement(ActivityView, {
        t: translations.ru,
        activeTransfers: [],
        history: [
          {
            id: "name-only-single",
            peerId: "peer-1",
            peerName: "SingleUriPeer",
            direction: "receive",
            fileCount: 1,
            totalBytes: 1024,
            status: "completed",
            savedPaths: ["content://provider/document/42"],
            errorMessage: null,
            createdAt: 1000,
          },
          {
            id: "name-only-multi",
            peerId: "peer-2",
            peerName: "MultiUriPeer",
            direction: "receive",
            fileCount: 2,
            totalBytes: 2048,
            status: "completed",
            savedPaths: [
              "content://provider/document/43",
              "content://provider/document/44",
            ],
            errorMessage: null,
            createdAt: 2000,
          },
        ],
        onPreviewPath: () => {},
        onError: () => {},
      })
    );
  });

  // Single record: both report.pdf and content://provider/document/42 must be visible
  assert.ok(
    container.textContent?.includes("report.pdf"),
    "single record must display resolved DISPLAY_NAME report.pdf"
  );
  assert.ok(
    container.textContent?.includes("content://provider/document/42"),
    "single record must display original content:// URI under report.pdf"
  );

  // Expand multi-file record: both filenames and original URIs must be visible
  const multiBtn = Array.from(container.querySelectorAll("button")).find((btn) =>
    btn.textContent?.includes("MultiUriPeer")
  );
  assert.ok(multiBtn, "multi-file record button must render");
  await act(async () => {
    multiBtn.click();
  });

  assert.ok(
    container.textContent?.includes("notes.txt"),
    "expanded list must display resolved DISPLAY_NAME notes.txt"
  );
  assert.ok(
    container.textContent?.includes("content://provider/document/43"),
    "expanded list must display original content:// URI for notes.txt"
  );
  assert.ok(
    container.textContent?.includes("content://provider/document/44"),
    "expanded list must display original content:// URI for report.pdf"
  );
});

test("ActivityView displays relative subfolder paths and opaque content:// URIs for sent folder with a/report.pdf and b/report.pdf", async (t) => {
  const ActivityView = await loadActivityViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "resolve_preview_names") {
      const paths = (args.paths as string[]) ?? [];
      const out: Record<string, string> = {};
      for (const p of paths) {
        if (
          p === "content://com.android.externalstorage.documents/document/101" ||
          p === "content://com.android.externalstorage.documents/document/102"
        ) {
          out[p] = "report.pdf";
        }
      }
      return out;
    }
    return {};
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  const uriA = "content://com.android.externalstorage.documents/document/101";
  const uriB = "content://com.android.externalstorage.documents/document/102";

  await act(async () => {
    root.render(
      React.createElement(ActivityView, {
        t: translations.ru,
        activeTransfers: [],
        history: [
          {
            id: "sent-opaque-folder",
            peerId: "peer-1",
            peerName: "OpaqueFolderPeer",
            direction: "send",
            fileCount: 2,
            totalBytes: 4096,
            status: "completed",
            savedPaths: [uriA, uriB],
            relativePaths: ["a/report.pdf", "b/report.pdf"],
            errorMessage: null,
            createdAt: 1000,
          },
        ],
        onPreviewPath: () => {},
        onError: () => {},
      })
    );
  });

  const rowBtn = Array.from(container.querySelectorAll("button")).find((btn) =>
    btn.textContent?.includes("OpaqueFolderPeer")
  );
  assert.ok(rowBtn, "sent folder row must be expandable");
  await act(async () => {
    rowBtn.click();
  });

  assert.ok(
    container.textContent?.includes("a/report.pdf"),
    "expanded sent folder must display relative path a/report.pdf"
  );
  assert.ok(
    container.textContent?.includes("b/report.pdf"),
    "expanded sent folder must display relative path b/report.pdf"
  );
  assert.ok(
    container.textContent?.includes(uriA),
    "expanded sent folder must display opaque URI for a/report.pdf"
  );
  assert.ok(
    container.textContent?.includes(uriB),
    "expanded sent folder must display opaque URI for b/report.pdf"
  );
});

test("summarizeTransferPaths and ActivityView prioritize stored relativePaths over commonDir stripping for files in the same nested folder", async (t) => {
  const summary = summarizeTransferPaths(
    ["/src/trip/day1/a.pdf", "/src/trip/day1/b.pdf"],
    {},
    ["day1/a.pdf", "day1/b.pdf"]
  );
  assert.deepEqual(
    summary.items.map((i) => i.relativePath),
    ["day1/a.pdf", "day1/b.pdf"]
  );

  const ActivityView = await loadActivityViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    env.cleanup();
  });

  await act(async () => {
    root.render(
      React.createElement(ActivityView, {
        t: translations.ru,
        activeTransfers: [],
        history: [
          {
            id: "same-nested-dir",
            peerId: "peer-1",
            peerName: "NestedDayPeer",
            direction: "send",
            fileCount: 2,
            totalBytes: 2048,
            status: "completed",
            savedPaths: ["/src/trip/day1/a.pdf", "/src/trip/day1/b.pdf"],
            relativePaths: ["day1/a.pdf", "day1/b.pdf"],
            errorMessage: null,
            createdAt: 1000,
          },
        ],
        onPreviewPath: () => {},
        onError: () => {},
      })
    );
  });

  const rowBtn = Array.from(container.querySelectorAll("button")).find((btn) =>
    btn.textContent?.includes("NestedDayPeer")
  );
  assert.ok(rowBtn, "nested folder row must be expandable");
  await act(async () => {
    rowBtn.click();
  });

  assert.ok(
    container.textContent?.includes("day1/a.pdf"),
    "expanded list must preserve day1/ prefix for a.pdf"
  );
  assert.ok(
    container.textContent?.includes("day1/b.pdf"),
    "expanded list must preserve day1/ prefix for b.pdf"
  );
  assert.ok(
    container.textContent?.includes("/src/trip/day1/a.pdf"),
    "expanded list must display full path for a.pdf"
  );
  assert.ok(
    container.textContent?.includes("/src/trip/day1/b.pdf"),
    "expanded list must display full path for b.pdf"
  );
});

