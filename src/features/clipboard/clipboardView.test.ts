import assert from "node:assert/strict";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import esbuild from "esbuild";
import { JSDOM } from "jsdom";
import type ReactType from "react";
import type {
  ClipboardItem,
  PeerInfo,
  PreviewDescriptor,
} from "../../core/types.ts";
import { translations } from "../../i18n/translations.ts";

const rootDir = path.resolve(import.meta.dirname, "../../..");
const require = createRequire(import.meta.url);

async function loadClipboardViewComponent() {
  const entryPath = path.join(rootDir, "src/features/clipboard/ClipboardView.tsx");
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
  const React = (await import("react")).default;
  const { act } = await import("react");
  const { createRoot } = await import("react-dom/client");
  return {
    React,
    act,
    createRoot,
    ClipboardView: mod.ClipboardView as ReactType.ComponentType<{
      t: (typeof translations)["ru"];
      peers: PeerInfo[];
      enabled: boolean;
      refreshTick: number;
      onPreviewDescriptor: (desc: PreviewDescriptor) => void;
      onError: (msg: string) => void;
      onNotice: (msg: string) => void;
    }>,
  };
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
    quicPort: 40406,
    isTrusted: true,
    isOnline: true,
    lastSeenAt: 1000,
    ...overrides,
  };
}

function makeTextItem(id: string, text: string): ClipboardItem {
  return {
    id,
    kind: "text",
    text,
    imagePath: null,
    imageWidth: null,
    imageHeight: null,
    sizeBytes: text.length,
    contentHash: `hash-${id}`,
    sourcePeerId: null,
    sourcePeerName: null,
    createdAt: 1000,
  };
}

function makeImageItem(id: string): ClipboardItem {
  return {
    id,
    kind: "image",
    text: "PNG 10×10",
    imagePath: `/tmp/${id}.png`,
    imageWidth: 10,
    imageHeight: 10,
    sizeBytes: 128,
    contentHash: `hash-${id}`,
    sourcePeerId: null,
    sourcePeerName: null,
    createdAt: 1000,
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
    HTMLInputElement: globalThis.HTMLInputElement,
    Node: globalThis.Node,
    Event: globalThis.Event,
    KeyboardEvent: globalThis.KeyboardEvent,
    IntersectionObserver: globalThis.IntersectionObserver,
    IS_REACT_ACT_ENVIRONMENT: (globalThis as Record<string, unknown>)
      .IS_REACT_ACT_ENVIRONMENT,
  };

  Object.assign(win, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    HTMLElement: win.HTMLElement,
    HTMLInputElement: win.HTMLInputElement,
    Node: win.Node,
    Event: win.Event,
    KeyboardEvent: win.KeyboardEvent,
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

async function flush(actFn: (cb: () => Promise<void>) => Promise<void>) {
  await actFn(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

function setInputValue(
  win: JSDOM["window"],
  input: HTMLInputElement,
  value: string
) {
  const setter = Object.getOwnPropertyDescriptor(
    win.HTMLInputElement.prototype,
    "value"
  )?.set;
  setter?.call(input, value);
  input.dispatchEvent(new win.Event("input", { bubbles: true }));
}

test("ClipboardView blocks duplicate text sends, refreshes target peer list, and isolates post-delivery errors", async (t) => {
  const env = setupDomEnvironment();
  const { React, act, createRoot, ClipboardView } = await loadClipboardViewComponent();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  let remoteItems: ClipboardItem[] = [makeTextItem("r-0", "initial")];
  let sendCalls = 0;
  let resolveSend: (() => void) | null = null;
  let failLocalSave = false;
  let failRemoteRefresh = false;
  const fetchedPeers: string[] = [];

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "clipboard_list_local") return [];
    if (cmd === "clipboard_fetch_remote") {
      const peerId = String(args.peerId);
      fetchedPeers.push(peerId);
      if (failRemoteRefresh) {
        throw new Error("refresh timeout");
      }
      return remoteItems;
    }
    if (cmd === "clipboard_send_to_peer") {
      sendCalls++;
      await new Promise<void>((resolve) => {
        resolveSend = resolve;
      });
      remoteItems = [
        makeTextItem(`r-${sendCalls}`, String(args.text)),
        ...remoteItems,
      ];
      return null;
    }
    if (cmd === "clipboard_push_local_text") {
      if (failLocalSave) {
        throw new Error("sqlite busy");
      }
      return makeTextItem("local-1", String(args.text));
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

  const errors: string[] = [];
  const notices: string[] = [];
  const peer = makePeer({ peerId: "peer-1", displayName: "Alice" });

  await act(async () => {
    root.render(
      React.createElement(ClipboardView, {
        t: translations.ru,
        peers: [peer],
        enabled: true,
        refreshTick: 0,
        onPreviewDescriptor: () => {},
        onError: (m) => errors.push(m),
        onNotice: (m) => notices.push(m),
      })
    );
  });
  await flush(act);

  // Select Alice
  const aliceBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Alice")
  )!;
  await act(async () => {
    aliceBtn.click();
  });
  await flush(act);
  assert.deepEqual(fetchedPeers, ["peer-1"]);

  const input = container.querySelector("input") as HTMLInputElement;
  const sendBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(translations.ru.clipboard.send)
  ) as HTMLButtonElement;

  await act(async () => {
    setInputValue(env.dom.window, input, "hello-alice");
  });

  // Trigger first send (stays in-flight)
  await act(async () => {
    sendBtn.click();
  });
  assert.equal(sendCalls, 1);
  assert.equal(sendBtn.disabled, true);

  // Attempt duplicate click and Enter while in-flight
  await act(async () => {
    sendBtn.click();
    input.dispatchEvent(
      new env.dom.window.KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
      })
    );
  });
  assert.equal(sendCalls, 1);

  // Complete delivery
  await act(async () => {
    resolveSend?.();
  });
  await flush(act);

  assert.equal(input.value, "");
  assert.deepEqual(notices, [translations.ru.clipboard.sent]);
  assert.deepEqual(fetchedPeers, ["peer-1", "peer-1"]);
  assert.ok(container.textContent?.includes("hello-alice"));
  assert.deepEqual(errors, []);

  // Second send: delivery succeeds, but local save and remote refresh fail -> draft still cleared, sent notice shown, distinct errors reported
  failLocalSave = true;
  failRemoteRefresh = true;
  await act(async () => {
    setInputValue(env.dom.window, input, "second-msg");
  });
  await act(async () => {
    sendBtn.click();
  });
  await act(async () => {
    resolveSend?.();
  });
  await flush(act);

  assert.equal(sendCalls, 2);
  assert.equal(input.value, "");
  assert.equal(notices.length, 2);
  assert.equal(errors.length, 2);
  assert.ok(errors[0].includes(translations.ru.clipboard.localSaveError));
  assert.ok(errors[1].includes(translations.ru.clipboard.refreshError));
});

test("ClipboardView blocks duplicate image sends and refreshes target peer list after delivery", async (t) => {
  const env = setupDomEnvironment();
  const { React, act, createRoot, ClipboardView } = await loadClipboardViewComponent();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  let remoteItems: ClipboardItem[] = [];
  let sendImageCalls = 0;
  let resolveImageSend: (() => void) | null = null;
  const fetchedPeers: string[] = [];

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "clipboard_list_local") return [];
    if (cmd === "clipboard_fetch_remote") {
      fetchedPeers.push(String(args.peerId));
      return remoteItems;
    }
    if (cmd === "clipboard_paste_from_system") {
      return { kind: "image", item: makeImageItem("img-local-1") };
    }
    if (cmd === "clipboard_send_image_to_peer") {
      sendImageCalls++;
      await new Promise<void>((resolve) => {
        resolveImageSend = resolve;
      });
      remoteItems = [makeImageItem("img-remote-1")];
      return null;
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

  const errors: string[] = [];
  const notices: string[] = [];
  const peer = makePeer({ peerId: "peer-img", displayName: "Bob" });

  await act(async () => {
    root.render(
      React.createElement(ClipboardView, {
        t: translations.ru,
        peers: [peer],
        enabled: true,
        refreshTick: 0,
        onPreviewDescriptor: () => {},
        onError: (m) => errors.push(m),
        onNotice: (m) => notices.push(m),
      })
    );
  });
  await flush(act);

  const bobBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Bob")
  )!;
  await act(async () => {
    bobBtn.click();
  });
  await flush(act);

  const pasteBtn = container.querySelector(
    `button[aria-label="${translations.ru.clipboard.pasteFromClipboard}"]`
  ) as HTMLButtonElement;
  await act(async () => {
    pasteBtn.click();
  });
  await flush(act);
  assert.ok(container.textContent?.includes("PNG 10×10"));

  const sendBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(translations.ru.clipboard.send)
  ) as HTMLButtonElement;

  await act(async () => {
    sendBtn.click();
  });
  assert.equal(sendImageCalls, 1);
  assert.equal(sendBtn.disabled, true);

  await act(async () => {
    sendBtn.click();
  });
  assert.equal(sendImageCalls, 1);

  await act(async () => {
    resolveImageSend?.();
  });
  await flush(act);

  assert.deepEqual(notices, [translations.ru.clipboard.sent]);
  assert.deepEqual(fetchedPeers, ["peer-img", "peer-img"]);
  assert.ok(container.textContent?.includes("PNG 10×10"));
  assert.deepEqual(errors, []);
});

test("ClipboardView preserves targetId across background peer updates and offline blink, disables send offline, and reloads on reconnect", async (t) => {
  const env = setupDomEnvironment();
  const { React, act, createRoot, ClipboardView } = await loadClipboardViewComponent();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  let fetchCount = 0;
  let sendCalls = 0;
  let remoteItems: ClipboardItem[] = [makeTextItem("r-1", "before-offline")];

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string
  ) => {
    if (cmd === "clipboard_list_local") return [];
    if (cmd === "clipboard_fetch_remote") {
      fetchCount++;
      return remoteItems;
    }
    if (cmd === "clipboard_send_to_peer") {
      sendCalls++;
      return null;
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

  const renderWithPeers = async (peers: PeerInfo[]) => {
    await act(async () => {
      root.render(
        React.createElement(ClipboardView, {
          t: translations.ru,
          peers,
          enabled: true,
          refreshTick: 0,
          onPreviewDescriptor: () => {},
          onError: () => {},
          onNotice: () => {},
        })
      );
    });
    await flush(act);
  };

  await renderWithPeers([makePeer({ peerId: "peer-1", displayName: "Alice", isOnline: true })]);

  const aliceBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Alice")
  )!;
  await act(async () => {
    aliceBtn.click();
  });
  await flush(act);
  assert.equal(fetchCount, 1);

  // Background peers update (new array reference, another peer added, Alice still online) -> no reset, no extra fetch
  await renderWithPeers([
    makePeer({ peerId: "peer-1", displayName: "Alice", isOnline: true, lastSeenAt: 2000 }),
    makePeer({ peerId: "peer-2", displayName: "Bob", isOnline: true }),
  ]);
  assert.equal(fetchCount, 1);
  assert.ok(container.textContent?.includes("before-offline"));

  // Alice goes temporarily offline -> targetId preserved, send disabled, offline banner shown
  await renderWithPeers([
    makePeer({ peerId: "peer-1", displayName: "Alice", isOnline: false }),
    makePeer({ peerId: "peer-2", displayName: "Bob", isOnline: true }),
  ]);
  assert.ok(container.textContent?.includes(translations.ru.clipboard.peerOffline));
  assert.ok(container.textContent?.includes("Alice"));

  const input = container.querySelector("input") as HTMLInputElement;
  await act(async () => {
    setInputValue(env.dom.window, input, "blocked-while-offline");
  });
  const sendBtnOffline = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(translations.ru.clipboard.send)
  ) as HTMLButtonElement;
  assert.equal(sendBtnOffline.disabled, true);
  await act(async () => {
    sendBtnOffline.click();
  });
  assert.equal(sendCalls, 0);

  // Alice comes back online -> remote list reloads automatically without manual re-selection
  remoteItems = [makeTextItem("r-2", "after-reconnect")];
  await renderWithPeers([
    makePeer({ peerId: "peer-1", displayName: "Alice", isOnline: true }),
    makePeer({ peerId: "peer-2", displayName: "Bob", isOnline: true }),
  ]);
  assert.equal(fetchCount, 2);
  assert.ok(container.textContent?.includes("after-reconnect"));
  assert.ok(!container.textContent?.includes(translations.ru.clipboard.peerOffline));

  // Alice is removed/untrusted -> resets cleanly to local tab
  await renderWithPeers([
    makePeer({ peerId: "peer-2", displayName: "Bob", isOnline: true }),
  ]);
  assert.ok(container.textContent?.includes(translations.ru.clipboard.saveLocal));
});

test("ClipboardView preserves draft edited while delivery is in-flight", async (t) => {
  const env = setupDomEnvironment();
  const { React, act, createRoot, ClipboardView } = await loadClipboardViewComponent();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  const sentTexts: string[] = [];
  let resolveFirstSend: (() => void) | null = null;
  let resolveSecondSend: (() => void) | null = null;

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "clipboard_list_local") return [];
    if (cmd === "clipboard_fetch_remote") return [];
    if (cmd === "clipboard_push_local_text") {
      return makeTextItem("local-1", String(args.text));
    }
    if (cmd === "clipboard_send_to_peer") {
      sentTexts.push(String(args.text));
      if (sentTexts.length === 1) {
        await new Promise<void>((resolve) => {
          resolveFirstSend = resolve;
        });
      } else {
        await new Promise<void>((resolve) => {
          resolveSecondSend = resolve;
        });
      }
      return null;
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
  const peer = makePeer({ peerId: "peer-1", displayName: "Alice" });

  await act(async () => {
    root.render(
      React.createElement(ClipboardView, {
        t: translations.ru,
        peers: [peer],
        enabled: true,
        refreshTick: 0,
        onPreviewDescriptor: () => {},
        onError: (m) => errors.push(m),
        onNotice: (m) => notices.push(m),
      })
    );
  });
  await flush(act);

  const aliceBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Alice")
  )!;
  await act(async () => {
    aliceBtn.click();
  });
  await flush(act);

  const input = container.querySelector("input") as HTMLInputElement;
  const sendBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(translations.ru.clipboard.send)
  ) as HTMLButtonElement;

  await act(async () => {
    setInputValue(env.dom.window, input, "first-msg");
  });
  await act(async () => {
    sendBtn.click();
  });
  assert.deepEqual(sentTexts, ["first-msg"]);

  // User edits draft while first delivery is still in-flight
  await act(async () => {
    setInputValue(env.dom.window, input, "next-msg");
  });
  assert.equal(input.value, "next-msg");

  // First delivery confirms -> edited draft must NOT be cleared
  await act(async () => {
    resolveFirstSend?.();
  });
  await flush(act);

  assert.equal(input.value, "next-msg");
  assert.deepEqual(notices, [translations.ru.clipboard.sent]);
  assert.equal(sendBtn.disabled, false);

  // User can immediately send the new draft
  await act(async () => {
    sendBtn.click();
  });
  assert.deepEqual(sentTexts, ["first-msg", "next-msg"]);
  await act(async () => {
    resolveSecondSend?.();
  });
  await flush(act);

  assert.equal(input.value, "");
  assert.deepEqual(errors, []);
});

test("ClipboardView displays peer B list when send to A confirms before B history resolves", async (t) => {
  const env = setupDomEnvironment();
  const { React, act, createRoot, ClipboardView } = await loadClipboardViewComponent();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  const fetchedPeers: string[] = [];
  let resolveSendA: (() => void) | null = null;
  let resolveFetchB: ((items: ClipboardItem[]) => void) | null = null;

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "clipboard_list_local") return [];
    if (cmd === "clipboard_push_local_text") {
      return makeTextItem("local-1", String(args.text));
    }
    if (cmd === "clipboard_send_to_peer") {
      await new Promise<void>((resolve) => {
        resolveSendA = resolve;
      });
      return null;
    }
    if (cmd === "clipboard_fetch_remote") {
      const peerId = String(args.peerId);
      fetchedPeers.push(peerId);
      if (peerId === "peer-a") {
        return [makeTextItem("a-1", "alice-item")];
      }
      if (peerId === "peer-b") {
        return await new Promise<ClipboardItem[]>((resolve) => {
          resolveFetchB = resolve;
        });
      }
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

  const errors: string[] = [];
  const notices: string[] = [];
  const peerA = makePeer({ peerId: "peer-a", displayName: "Alice" });
  const peerB = makePeer({ peerId: "peer-b", displayName: "Bob" });

  await act(async () => {
    root.render(
      React.createElement(ClipboardView, {
        t: translations.ru,
        peers: [peerA, peerB],
        enabled: true,
        refreshTick: 0,
        onPreviewDescriptor: () => {},
        onError: (m) => errors.push(m),
        onNotice: (m) => notices.push(m),
      })
    );
  });
  await flush(act);

  // Select Alice and wait for A's list
  const aliceBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Alice")
  )!;
  await act(async () => {
    aliceBtn.click();
  });
  await flush(act);
  assert.deepEqual(fetchedPeers, ["peer-a"]);
  assert.ok(container.textContent?.includes("alice-item"));

  // Start sending to Alice (stays in-flight)
  const input = container.querySelector("input") as HTMLInputElement;
  const sendBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(translations.ru.clipboard.send)
  ) as HTMLButtonElement;
  await act(async () => {
    setInputValue(env.dom.window, input, "msg-for-a");
  });
  await act(async () => {
    sendBtn.click();
  });

  // Switch to Bob while send to A is still in-flight -> B's fetch starts and stays pending
  const bobBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Bob")
  )!;
  await act(async () => {
    bobBtn.click();
  });
  await flush(act);
  assert.deepEqual(fetchedPeers, ["peer-a", "peer-b"]);

  // Send to A confirms BEFORE B's history resolves -> must NOT trigger fetch for A or invalidate B
  await act(async () => {
    resolveSendA?.();
  });
  await flush(act);
  assert.deepEqual(fetchedPeers, ["peer-a", "peer-b"]);
  assert.deepEqual(notices, [translations.ru.clipboard.sent]);

  // Now B's history resolves -> Bob's items must appear without manual refresh
  await act(async () => {
    resolveFetchB?.([makeTextItem("b-1", "bob-clipboard-item")]);
  });
  await flush(act);

  assert.ok(container.textContent?.includes("bob-clipboard-item"));
  assert.ok(!container.textContent?.includes("alice-item"));
  assert.deepEqual(errors, []);
});

test("ClipboardView pastes text/image from system clipboard and copies items back to system clipboard", async (t) => {
  const env = setupDomEnvironment();
  const { React, act, createRoot, ClipboardView } = await loadClipboardViewComponent();
  const container = env.dom.window.document.getElementById("root")!;
  const root = createRoot(container);

  const prevMock = (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__;
  let pasteCall = 0;
  let localItems: ClipboardItem[] = [makeTextItem("item-1", "existing-text")];
  const copiedCalls: Array<{ itemId: string; peerId: string | null; text: string | null }> = [];
  let failNextCopy = false;

  (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = async (
    cmd: string,
    args: Record<string, unknown>
  ) => {
    if (cmd === "clipboard_list_local") return localItems;
    if (cmd === "clipboard_paste_from_system") {
      pasteCall += 1;
      if (pasteCall === 1) {
        return { kind: "text", text: "pasted-from-os" };
      }
      const imgItem = makeImageItem("img-pasted");
      localItems = [imgItem, ...localItems];
      return { kind: "image", item: imgItem };
    }
    if (cmd === "clipboard_copy_to_system") {
      if (failNextCopy) {
        throw new Error("System clipboard unavailable");
      }
      copiedCalls.push({
        itemId: String(args.itemId),
        peerId: (args.peerId as string | null) ?? null,
        text: (args.text as string | null) ?? null,
      });
      return null;
    }
    if (cmd === "clipboard_get_image_preview") {
      return { url: "http://127.0.0.1:1234/preview.png", mime: "image/png" };
    }
    return null;
  };

  const errors: string[] = [];
  const notices: string[] = [];

  t.after(async () => {
    await act(async () => {
      root.unmount();
    });
    (globalThis as Record<string, unknown>).__TAURI_INVOKE_MOCK__ = prevMock;
    env.cleanup();
  });

  await act(async () => {
    root.render(
      React.createElement(ClipboardView, {
        t: translations.ru,
        peers: [],
        enabled: true,
        refreshTick: 0,
        onPreviewDescriptor: () => {},
        onError: (m) => errors.push(m),
        onNotice: (m) => notices.push(m),
      })
    );
  });
  await flush(act);

  const pasteBtn = container.querySelector(
    `button[aria-label="${translations.ru.clipboard.pasteFromClipboard}"]`
  ) as HTMLButtonElement;
  assert.ok(pasteBtn);

  // 1. Paste text from system clipboard -> populates input
  await act(async () => {
    pasteBtn.click();
  });
  await flush(act);
  const input = container.querySelector("input") as HTMLInputElement;
  assert.equal(input.value, "pasted-from-os");

  // 2. Paste image from system clipboard -> refreshes local list with image item
  await act(async () => {
    pasteBtn.click();
  });
  await flush(act);
  assert.ok(container.textContent?.includes("PNG 10×10"));

  // 3. Copy item to system clipboard -> invokes clipboard_copy_to_system and shows notice
  const copyButtons = Array.from(container.querySelectorAll("button")).filter((b) =>
    b.querySelector("svg.lucide-copy")
  ) as HTMLButtonElement[];
  assert.ok(copyButtons.length >= 1);

  await act(async () => {
    copyButtons[0].click();
  });
  await flush(act);
  assert.equal(copiedCalls.length, 1);
  assert.equal(copiedCalls[0].itemId, "img-pasted");
  assert.deepEqual(notices, [translations.ru.clipboard.copied]);

  // 4. Copy failure surfaces error without crashing
  failNextCopy = true;
  await act(async () => {
    copyButtons[0].click();
  });
  await flush(act);
  assert.equal(errors.length, 1);
  assert.ok(errors[0].includes("System clipboard unavailable"));
});
