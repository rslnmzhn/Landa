import assert from "node:assert/strict";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import esbuild from "esbuild";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import type { PeerInfo, TransferProgressInfo } from "../../core/types.ts";
import { translations } from "../../i18n/translations.ts";

const rootDir = path.resolve(import.meta.dirname, "../../..");
const require = createRequire(import.meta.url);

async function loadSharesViewComponent() {
  const entryPath = path.join(rootDir, "src/features/shares/SharesView.tsx");
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
  return mod.SharesView as React.ComponentType<{
    t: (typeof translations)["ru"];
    identity: null;
    peers: PeerInfo[];
    enabled: boolean;
    activeTransfers?: TransferProgressInfo[];
    downloadingKey?: string | null;
    onDownloadingKeyChange?: React.Dispatch<React.SetStateAction<string | null>>;
    onPreviewRemoteFile: (peerId: string, rootId: string, relPath: string) => void;
    onPreviewLocalPath: (path: string) => void;
    onError: (msg: string) => void;
    onNotice: (msg: string) => void;
  }>;
}

function makePeer(overrides: Partial<PeerInfo> = {}): PeerInfo {
  return {
    peerId: "peer-1",
    publicKeyHex: "00".repeat(32),
    sasCode: "123456",
    deviceName: "Alice",
    aliasName: null,
    displayName: "Alice",
    os: "linux",
    category: "desktop",
    ip: "192.168.1.10",
    quicPort: 40405,
    isTrusted: true,
    isOnline: true,
    lastSeenAt: 1000,
    ...overrides,
  };
}

function setupDomEnvironment() {
  const dom = new JSDOM(
    '<!doctype html><html><body><div id="root"></div></body></html>',
    { url: "http://localhost/" }
  );
  const win = dom.window;

  class NoopIntersectionObserver {
    observe() {}
    disconnect() {}
    unobserve() {}
  }

  const prevGlobals = {
    window: globalThis.window,
    document: globalThis.document,
    HTMLElement: globalThis.HTMLElement,
    Node: globalThis.Node,
    IntersectionObserver: globalThis.IntersectionObserver,
    IS_REACT_ACT_ENVIRONMENT: (globalThis as Record<string, unknown>)
      .IS_REACT_ACT_ENVIRONMENT,
  };

  Object.assign(win, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    HTMLElement: win.HTMLElement,
    Node: win.Node,
    IntersectionObserver: NoopIntersectionObserver,
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

test("SharesView omits Eye button in list and grid modes while keeping card/row preview and download", async (t) => {
  const SharesView = await loadSharesViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  const downloadedCalls: Array<{ peerId: string; rootId: string; relPath: string }> = [];
  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "shares_list_local") return [];
    if (cmd === "shares_browse_remote_roots") {
      return [{ id: "root-1", name: "Photos", isDir: true }];
    }
    if (cmd === "shares_browse_remote_dir") {
      return [
        {
          name: "vacation.jpg",
          relPath: "vacation.jpg",
          isDir: false,
          sizeBytes: 2048,
          modifiedMs: 1000,
        },
      ];
    }
    if (cmd === "shares_download_remote_file") {
      downloadedCalls.push({
        peerId: String(args.peerId),
        rootId: String(args.rootId),
        relPath: String(args.relPath),
      });
      return "/tmp/vacation.jpg";
    }
    return null;
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  const previewCalls: Array<{ peerId: string; rootId: string; relPath: string }> = [];
  const notices: string[] = [];
  const errors: string[] = [];

  await act(async () => {
    root.render(
      React.createElement(SharesView, {
        t: translations.ru,
        identity: null,
        peers: [makePeer()],
        enabled: true,
        onPreviewRemoteFile: (peerId, rootId, relPath) => {
          previewCalls.push({ peerId, rootId, relPath });
        },
        onPreviewLocalPath: () => {},
        onError: (msg) => errors.push(msg),
        onNotice: (msg) => notices.push(msg),
      })
    );
  });

  const findButtonByText = (text: string) =>
    Array.from(container.querySelectorAll("button")).find((btn) =>
      btn.textContent?.includes(text)
    );

  const peerBtn = findButtonByText("Alice");
  assert.ok(peerBtn, "friend selector button should render");
  await act(async () => {
    peerBtn.click();
  });

  const rootBtn = findButtonByText("Photos");
  assert.ok(rootBtn, "remote root button should render");
  await act(async () => {
    rootBtn.click();
  });

  // List mode assertions
  assert.equal(
    container.querySelectorAll("svg.lucide-eye").length,
    0,
    "list mode must not render Eye icon"
  );
  const listFileBtn = findButtonByText("vacation.jpg");
  assert.ok(listFileBtn, "file row button should render in list mode");
  await act(async () => {
    listFileBtn.click();
  });
  assert.deepEqual(previewCalls, [
    { peerId: "peer-1", rootId: "root-1", relPath: "vacation.jpg" },
  ]);

  const listDownloadIcon = container.querySelector("svg.lucide-download");
  const listDownloadBtn = listDownloadIcon?.closest("button");
  assert.ok(listDownloadBtn, "download button should remain in list mode");
  await act(async () => {
    listDownloadBtn.click();
  });
  assert.equal(downloadedCalls.length, 1);

  // Switch to Grid mode
  const gridToggleBtn = findButtonByText(translations.ru.shares.viewGrid);
  assert.ok(gridToggleBtn, "grid view toggle should render");
  await act(async () => {
    gridToggleBtn.click();
  });

  assert.equal(
    container.querySelectorAll("svg.lucide-eye").length,
    0,
    "grid mode must not render Eye icon"
  );
  const gridFileBtn = findButtonByText("vacation.jpg");
  assert.ok(gridFileBtn, "file card button should render in grid mode");
  await act(async () => {
    gridFileBtn.click();
  });
  assert.equal(previewCalls.length, 2);

  const gridDownloadIcon = container.querySelector("svg.lucide-download");
  const gridDownloadBtn = gridDownloadIcon?.closest("button");
  assert.ok(gridDownloadBtn, "download button should remain in grid mode");
  await act(async () => {
    gridDownloadBtn.click();
  });
  assert.equal(downloadedCalls.length, 2);
  assert.deepEqual(errors, []);
  assert.equal(notices.length, 2);
});

test("SharesView renders download buttons for remote roots and folders in list and grid modes", async (t) => {
  const SharesView = await loadSharesViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  const folderDownloads: Array<{ peerId: string; rootId: string; relPath: string }> = [];
  const fileDownloads: Array<{ peerId: string; rootId: string; relPath: string }> = [];
  const browsedDirs: string[] = [];

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "shares_list_local") return [];
    if (cmd === "shares_browse_remote_roots") {
      return [
        { id: "root-dir", name: "Photos", isDir: true },
        { id: "root-file", name: "readme.txt", isDir: false },
      ];
    }
    if (cmd === "shares_browse_remote_dir") {
      const rel = String(args.relPath ?? "");
      browsedDirs.push(rel);
      if (rel === "") {
        return [
          {
            name: "Summer",
            relPath: "Summer",
            isDir: true,
            sizeBytes: 0,
            modifiedMs: 1000,
          },
        ];
      }
      return [];
    }
    if (cmd === "shares_download_remote_folder") {
      folderDownloads.push({
        peerId: String(args.peerId),
        rootId: String(args.rootId),
        relPath: String(args.relPath),
      });
      return {
        folderName: String(args.relPath || "Photos"),
        fileCount: 3,
        totalBytes: 4096,
        savedPaths: ["/tmp/a.jpg"],
        destination: "/tmp/Photos",
      };
    }
    if (cmd === "shares_download_remote_file") {
      fileDownloads.push({
        peerId: String(args.peerId),
        rootId: String(args.rootId),
        relPath: String(args.relPath),
      });
      return "/tmp/readme.txt";
    }
    return null;
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  const notices: string[] = [];
  const errors: string[] = [];

  await act(async () => {
    root.render(
      React.createElement(SharesView, {
        t: translations.ru,
        identity: null,
        peers: [makePeer()],
        enabled: true,
        onPreviewRemoteFile: () => {},
        onPreviewLocalPath: () => {},
        onError: (msg) => errors.push(msg),
        onNotice: (msg) => notices.push(msg),
      })
    );
  });

  const findButtonByText = (text: string) =>
    Array.from(container.querySelectorAll("button")).find((btn) =>
      btn.textContent?.includes(text)
    );

  const peerBtn = findButtonByText("Alice");
  assert.ok(peerBtn);
  await act(async () => {
    peerBtn.click();
  });

  // Root cards should have download buttons for both folder root and file root
  const rootDownloadBtns = Array.from(
    container.querySelectorAll("svg.lucide-download")
  ).map((svg) => svg.closest("button")!);
  assert.equal(rootDownloadBtns.length, 2, "each remote root should have a download button");

  await act(async () => {
    rootDownloadBtns[0].click();
  });
  assert.deepEqual(folderDownloads, [
    { peerId: "peer-1", rootId: "root-dir", relPath: "" },
  ]);

  await act(async () => {
    rootDownloadBtns[1].click();
  });
  assert.deepEqual(fileDownloads, [
    { peerId: "peer-1", rootId: "root-file", relPath: "" },
  ]);

  // Open folder root "Photos"
  const photosBtn = findButtonByText("Photos");
  assert.ok(photosBtn);
  await act(async () => {
    photosBtn.click();
  });

  // List mode: folder "Summer" should have a download button
  const listFolderDownloadBtn = container
    .querySelector("svg.lucide-download")
    ?.closest("button");
  assert.ok(listFolderDownloadBtn, "folder row in list mode must have download button");
  await act(async () => {
    listFolderDownloadBtn.click();
  });
  assert.deepEqual(folderDownloads[1], {
    peerId: "peer-1",
    rootId: "root-dir",
    relPath: "Summer",
  });

  // Switch to Grid mode: folder "Summer" should also have a download button
  const gridToggleBtn = findButtonByText(translations.ru.shares.viewGrid);
  assert.ok(gridToggleBtn);
  await act(async () => {
    gridToggleBtn.click();
  });

  const gridFolderDownloadBtn = container
    .querySelector("svg.lucide-download")
    ?.closest("button");
  assert.ok(gridFolderDownloadBtn, "folder card in grid mode must have download button");
  await act(async () => {
    gridFolderDownloadBtn.click();
  });
  assert.deepEqual(folderDownloads[2], {
    peerId: "peer-1",
    rootId: "root-dir",
    relPath: "Summer",
  });

  // Clicking the folder card itself still navigates into "Summer"
  const summerCardBtn = findButtonByText("Summer");
  assert.ok(summerCardBtn);
  await act(async () => {
    summerCardBtn.click();
  });
  assert.ok(browsedDirs.includes("Summer"));
  assert.deepEqual(errors, []);
  assert.equal(notices.length, 4);
});

test("SharesView locks concurrent downloads, shows spinner, recovers on error, and renders active receive progress", async (t) => {
  const SharesView = await loadSharesViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  let resolveFolderDownload: ((val: unknown) => void) | null = null;
  let rejectFolderDownload: ((err: unknown) => void) | null = null;
  let folderDownloadCalls = 0;

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string
  ) => {
    if (cmd === "shares_list_local") {
      return [
        { id: "local-1", name: "MyDocs", path: "/tmp/MyDocs", isDir: true },
      ];
    }
    if (cmd === "shares_remove_local") {
      throw new Error("remove failed");
    }
    if (cmd === "shares_browse_remote_roots") {
      return [
        { id: "root-dir", name: "Photos", isDir: true },
        { id: "root-file", name: "readme.txt", isDir: false },
      ];
    }
    if (cmd === "shares_download_remote_folder") {
      folderDownloadCalls++;
      return new Promise((resolve, reject) => {
        resolveFolderDownload = resolve;
        rejectFolderDownload = reject;
      });
    }
    return null;
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  const notices: string[] = [];
  const errors: string[] = [];

  const activeTransfers: TransferProgressInfo[] = [
    {
      transferId: "tr-send",
      direction: "send",
      peerId: "peer-1",
      peerName: "Alice",
      currentFile: "outgoing.zip",
      filesDone: 0,
      fileCount: 1,
      bytesTransferred: 50,
      totalBytes: 100,
      speedBps: 10,
      etaSecs: 5,
      status: "transferring",
      error: null,
    },
    {
      transferId: "tr-wait",
      direction: "receive",
      peerId: "peer-1",
      peerName: "Alice",
      currentFile: "pending.zip",
      filesDone: 0,
      fileCount: 1,
      bytesTransferred: 0,
      totalBytes: 100,
      speedBps: 0,
      etaSecs: null,
      status: "waiting_approval",
      error: null,
    },
    {
      transferId: "tr-recv-zero",
      direction: "receive",
      peerId: "peer-1",
      peerName: "Alice",
      currentFile: "Photos/empty.txt",
      filesDone: 0,
      fileCount: 2,
      bytesTransferred: 0,
      totalBytes: 0,
      speedBps: 0,
      etaSecs: null,
      status: "transferring",
      error: null,
    },
    {
      transferId: "tr-recv-half",
      direction: "receive",
      peerId: "peer-1",
      peerName: "Alice",
      currentFile: "Photos/pic.jpg",
      filesDone: 1,
      fileCount: 2,
      bytesTransferred: 512,
      totalBytes: 1024,
      speedBps: 256,
      etaSecs: 2,
      status: "transferring",
      error: null,
    },
  ];

  await act(async () => {
    root.render(
      React.createElement(SharesView, {
        t: translations.ru,
        identity: null,
        peers: [makePeer()],
        enabled: true,
        activeTransfers,
        onPreviewRemoteFile: () => {},
        onPreviewLocalPath: () => {},
        onError: (msg) => errors.push(msg),
        onNotice: (msg) => notices.push(msg),
      })
    );
  });

  // Local share delete button has aria-label and surfaces errors via onError
  const removeBtn = container.querySelector(
    `button[aria-label="${translations.ru.shares.remove}: MyDocs"]`
  ) as HTMLButtonElement | null;
  assert.ok(removeBtn, "local share delete button must have aria-label");
  await act(async () => {
    removeBtn.click();
  });
  assert.equal(errors.length, 1);
  assert.match(errors[0], /remove failed/);

  // Active receive transfers render only transferring receive items with progressbar semantics
  const progressBars = Array.from(
    container.querySelectorAll('[role="progressbar"]')
  );
  assert.equal(progressBars.length, 2);
  assert.equal(progressBars[0].getAttribute("aria-valuenow"), "0");
  assert.equal(progressBars[1].getAttribute("aria-valuenow"), "50");
  assert.ok(!container.textContent?.includes("outgoing.zip"));
  assert.ok(!container.textContent?.includes("pending.zip"));
  assert.ok(container.textContent?.includes("Photos/pic.jpg"));

  // Select peer and inspect root download button labels
  const peerBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Alice")
  );
  assert.ok(peerBtn);
  await act(async () => {
    peerBtn.click();
  });

  const folderBtn = container.querySelector(
    `button[aria-label="${translations.ru.shares.downloadFolder}: Photos"]`
  ) as HTMLButtonElement | null;
  const fileBtn = container.querySelector(
    `button[aria-label="${translations.ru.shares.download}: readme.txt"]`
  ) as HTMLButtonElement | null;
  assert.ok(folderBtn, "folder root button must use downloadFolder label");
  assert.ok(fileBtn, "file root button must use download label");
  assert.equal(folderBtn.getAttribute("title"), translations.ru.shares.downloadFolder);
  assert.equal(fileBtn.getAttribute("title"), translations.ru.shares.download);

  // Start deferred folder download and double-click synchronously
  await act(async () => {
    folderBtn.click();
    folderBtn.click();
  });
  assert.equal(folderDownloadCalls, 1, "synchronous guard must prevent duplicate download");
  assert.equal(folderBtn.disabled, true);
  assert.equal(fileBtn.disabled, true);
  assert.ok(folderBtn.querySelector(".animate-spin"), "active download button must show spinner");

  // Resolve first download and verify notice + re-enabled buttons
  await act(async () => {
    resolveFolderDownload?.({
      folderName: "Photos",
      fileCount: 4,
      totalBytes: 8192,
      savedPaths: ["/tmp/Photos/a.jpg"],
      destination: "Downloads/Landa/Photos",
    });
  });
  assert.equal(folderBtn.disabled, false);
  assert.equal(fileBtn.disabled, false);
  assert.equal(folderBtn.querySelector(".animate-spin"), null);
  assert.equal(notices.length, 1);
  assert.ok(notices[0].includes("Downloads/Landa/Photos"));
  assert.ok(notices[0].includes("4"));

  // Start second download and reject it to verify error recovery
  await act(async () => {
    folderBtn.click();
  });
  assert.equal(folderBtn.disabled, true);
  await act(async () => {
    rejectFolderDownload?.(new Error("network timeout"));
  });
  assert.equal(folderBtn.disabled, false);
  assert.equal(fileBtn.disabled, false);
  assert.equal(errors.length, 2);
  assert.match(errors[1], /network timeout/);
});

test("SharesView displays local share paths/URIs and clickable multi-level breadcrumbs", async (t) => {
  const SharesView = await loadSharesViewComponent();
  const env = setupDomEnvironment();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  const browsedDirs: string[] = [];

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "shares_list_local") {
      return [
        {
          id: "local-fs",
          name: "WorkDocs",
          path: "C:\\Users\\alice\\Documents\\VeryLongFolderName\\WorkDocs",
          isDir: true,
        },
        {
          id: "local-saf",
          name: "Camera",
          path: "content://com.android.externalstorage.documents/tree/primary%3ADCIM%2FCamera",
          isDir: true,
        },
        {
          id: "local-file",
          name: "notes.txt",
          path: "/home/alice/notes.txt",
          isDir: false,
        },
      ];
    }
    if (cmd === "shares_browse_remote_roots") {
      return [{ id: "root-1", name: "Photos", isDir: true }];
    }
    if (cmd === "shares_browse_remote_dir") {
      const rel = String(args.relPath ?? "");
      browsedDirs.push(rel);
      if (rel === "") {
        return [
          {
            name: "2026",
            relPath: "2026",
            isDir: true,
            sizeBytes: 0,
            modifiedMs: 1000,
          },
        ];
      }
      if (rel === "2026") {
        return [
          {
            name: "Summer",
            relPath: "2026/Summer",
            isDir: true,
            sizeBytes: 0,
            modifiedMs: 1000,
          },
        ];
      }
      if (rel === "2026/Summer") {
        return [
          {
            name: "August",
            relPath: "2026/Summer/August",
            isDir: true,
            sizeBytes: 0,
            modifiedMs: 1000,
          },
        ];
      }
      return [];
    }
    return null;
  };

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  const localPreviewCalls: string[] = [];

  await act(async () => {
    root.render(
      React.createElement(SharesView, {
        t: translations.ru,
        identity: null,
        peers: [makePeer()],
        enabled: true,
        onPreviewRemoteFile: () => {},
        onPreviewLocalPath: (p) => localPreviewCalls.push(p),
        onError: () => {},
        onNotice: () => {},
      })
    );
  });

  // Local shares must display full path and content:// URI
  assert.ok(
    container.textContent?.includes(
      "C:\\Users\\alice\\Documents\\VeryLongFolderName\\WorkDocs"
    ),
    "local filesystem share must display full path"
  );
  assert.ok(
    container.textContent?.includes(
      "content://com.android.externalstorage.documents/tree/primary%3ADCIM%2FCamera"
    ),
    "local SAF share must display content:// URI"
  );
  assert.ok(
    container.textContent?.includes("/home/alice/notes.txt"),
    "local file share must display full path"
  );

  // Local folder shares must not render as disabled <button>
  const disabledLocalFolderBtn = Array.from(
    container.querySelectorAll("button[disabled]")
  ).find((btn) => btn.textContent?.includes("WorkDocs"));
  assert.equal(
    disabledLocalFolderBtn,
    undefined,
    "local folder share must not render as disabled button"
  );

  const findButtonByText = (text: string) =>
    Array.from(container.querySelectorAll("button")).find((btn) =>
      btn.textContent?.includes(text)
    );

  // Clicking local file share triggers onPreviewLocalPath
  const localFileBtn = findButtonByText("notes.txt");
  assert.ok(localFileBtn, "local file share must be clickable");
  await act(async () => {
    localFileBtn.click();
  });
  assert.deepEqual(localPreviewCalls, ["/home/alice/notes.txt"]);

  // Navigate down 3 levels: Photos -> 2026 -> Summer -> August
  await act(async () => {
    findButtonByText("Alice")!.click();
  });
  await act(async () => {
    findButtonByText("Photos")!.click();
  });
  await act(async () => {
    findButtonByText("2026")!.click();
  });
  await act(async () => {
    findButtonByText("Summer")!.click();
  });
  await act(async () => {
    findButtonByText("August")!.click();
  });

  const nav = container.querySelector("nav[aria-label]");
  assert.ok(nav, "breadcrumbs must render inside a <nav> landmark");
  assert.ok(nav.querySelector("ol"), "breadcrumbs must use <ol> list semantics");
  assert.ok(nav.textContent?.includes("Photos"));
  assert.ok(nav.textContent?.includes("2026"));
  assert.ok(nav.textContent?.includes("Summer"));
  assert.ok(nav.textContent?.includes("August"));

  const currentCrumb = nav.querySelector('[aria-current="page"]');
  assert.ok(currentCrumb, "active breadcrumb must have aria-current='page'");
  assert.equal(currentCrumb.tagName, "SPAN");
  assert.equal(currentCrumb.textContent?.trim(), "August");

  // Breadcrumb segments must not be clipped with max-w-[180px] truncate
  assert.equal(
    nav.querySelector(".max-w-\\[180px\\]"),
    null,
    "breadcrumbs must not truncate inside narrow max-w-[180px] badge"
  );

  // Back button navigates up one level (2026/Summer/August -> 2026/Summer)
  const backBtn = findButtonByText(translations.ru.shares.back);
  assert.ok(backBtn, "Back button must render");
  await act(async () => {
    backBtn.click();
  });
  assert.equal(browsedDirs[browsedDirs.length - 1], "2026/Summer");

  // Click intermediate breadcrumb "2026" to jump directly up
  const navAtSummer = container.querySelector("nav[aria-label]")!;
  const crumb2026 = Array.from(navAtSummer.querySelectorAll("button")).find(
    (btn) => btn.textContent?.trim() === "2026"
  );
  assert.ok(crumb2026, "intermediate breadcrumb '2026' must be clickable");
  await act(async () => {
    crumb2026.click();
  });
  assert.equal(browsedDirs[browsedDirs.length - 1], "2026");

  // Click root breadcrumb "Photos" to jump directly to share root
  const navAfterJump = container.querySelector("nav[aria-label]");
  assert.ok(navAfterJump, "breadcrumbs must remain rendered after jumping up");
  const crumbRoot = Array.from(navAfterJump.querySelectorAll("button")).find(
    (btn) => btn.textContent?.trim() === "Photos"
  );
  assert.ok(crumbRoot, "root breadcrumb 'Photos' must be clickable");
  await act(async () => {
    crumbRoot.click();
  });
  assert.equal(browsedDirs[browsedDirs.length - 1], "");
  const rootCurrent = container.querySelector('nav[aria-label] [aria-current="page"]');
  assert.equal(rootCurrent?.textContent?.trim(), "Photos");

  // When selected peer goes offline, remote browse state resets cleanly
  await act(async () => {
    root.render(
      React.createElement(SharesView, {
        t: translations.ru,
        identity: null,
        peers: [makePeer({ isOnline: false })],
        enabled: true,
        onPreviewRemoteFile: () => {},
        onPreviewLocalPath: (p) => localPreviewCalls.push(p),
        onError: () => {},
        onNotice: () => {},
      })
    );
  });
  assert.ok(
    container.textContent?.includes(translations.ru.shares.noFriends),
    "going offline must reset remote browse state and show noFriends notice"
  );
});
