import assert from "node:assert/strict";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import esbuild from "esbuild";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { translations } from "../../i18n/translations.ts";
import {
  DOUBLE_TAP_DELAY_MS,
  DOUBLE_TAP_SCALE,
  MAX_IMAGE_SCALE,
  MIN_IMAGE_SCALE,
  clampOffset,
  getPinchMetrics,
  isDoubleTapCandidate,
  panByDelta,
  toggleDoubleTapZoom,
  zoomAroundPoint,
} from "./imageGestures.ts";

const rootDir = path.resolve(import.meta.dirname, "../../..");
const require = createRequire(import.meta.url);

async function loadFilePreviewModalComponent() {
  const entryPath = path.join(
    rootDir,
    "src/features/preview/FilePreviewModal.tsx"
  );
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
          build.onResolve({ filter: /^@tauri-apps\/api\// }, (args) => ({
            path: args.path,
            namespace: "tauri-stub",
          }));
          build.onLoad({ filter: /.*/, namespace: "tauri-stub" }, () => ({
            contents: `
              export async function invoke() { return null; }
              export async function listen() { return () => {}; }
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
  return mod.FilePreviewModal as React.ComponentType<{
    t: (typeof translations)["ru"];
    preview: {
      kind: "image" | "video" | "audio" | "text" | "unsupported";
      fileName: string;
      mimeType: string;
      sizeBytes: number;
      url: string | null;
      textContent: string | null;
      truncated: boolean;
      sourcePath: string;
    } | null;
    loading: boolean;
    onClose: () => void;
    onError: (msg: string) => void;
    onNotice: (msg: string) => void;
  }>;
}

function setupDomEnvironment() {
  const dom = new JSDOM(
    '<!doctype html><html><body><div id="root"></div></body></html>',
    { url: "http://localhost/" }
  );
  const win = dom.window;

  class PolyfillPointerEvent extends win.MouseEvent {
    pointerId: number;
    pointerType: string;
    constructor(
      type: string,
      init: MouseEventInit & { pointerId?: number; pointerType?: string } = {}
    ) {
      super(type, { bubbles: true, cancelable: true, ...init });
      this.pointerId = init.pointerId ?? 1;
      this.pointerType = init.pointerType ?? "touch";
    }
  }

  const captured = new Map<EventTarget, Set<number>>();
  win.HTMLElement.prototype.setPointerCapture = function (pointerId: number) {
    let set = captured.get(this);
    if (!set) {
      set = new Set();
      captured.set(this, set);
    }
    set.add(pointerId);
  };
  win.HTMLElement.prototype.hasPointerCapture = function (pointerId: number) {
    return captured.get(this)?.has(pointerId) ?? false;
  };
  win.HTMLElement.prototype.releasePointerCapture = function (
    pointerId: number
  ) {
    captured.get(this)?.delete(pointerId);
  };

  const prevGlobals = {
    window: globalThis.window,
    document: globalThis.document,
    HTMLElement: globalThis.HTMLElement,
    Element: globalThis.Element,
    Node: globalThis.Node,
    Event: globalThis.Event,
    MouseEvent: globalThis.MouseEvent,
    PointerEvent: globalThis.PointerEvent,
    WheelEvent: globalThis.WheelEvent,
    KeyboardEvent: globalThis.KeyboardEvent,
    IS_REACT_ACT_ENVIRONMENT: (globalThis as Record<string, unknown>)
      .IS_REACT_ACT_ENVIRONMENT,
  };

  Object.assign(win, {
    IS_REACT_ACT_ENVIRONMENT: true,
  });

  Object.assign(globalThis, {
    window: win,
    document: win.document,
    HTMLElement: win.HTMLElement,
    Element: win.Element,
    Node: win.Node,
    Event: win.Event,
    MouseEvent: win.MouseEvent,
    PointerEvent: PolyfillPointerEvent,
    WheelEvent: win.WheelEvent,
    KeyboardEvent: win.KeyboardEvent,
    IS_REACT_ACT_ENVIRONMENT: true,
  });

  return {
    dom,
    PointerEventCtor: PolyfillPointerEvent,
    cleanup() {
      Object.assign(globalThis, prevGlobals);
      dom.window.close();
    },
  };
}

function parseScaleFromTransform(transform: string): number {
  const match = transform.match(/scale\(([^)]+)\)/);
  return match ? Number(match[1]) : 1;
}

function parseTranslateFromTransform(transform: string): { x: number; y: number } {
  const match = transform.match(/translate3d\(([^,]+)px,\s*([^,]+)px,/);
  return match ? { x: Number(match[1]), y: Number(match[2]) } : { x: 0, y: 0 };
}

function dispatchTap(
  target: Element,
  PointerEventCtor: new (
    type: string,
    init?: MouseEventInit & { pointerId?: number; pointerType?: string }
  ) => Event,
  x: number,
  y: number,
  pointerId = 1
) {
  target.dispatchEvent(
    new PointerEventCtor("pointerdown", {
      clientX: x,
      clientY: y,
      pointerId,
      pointerType: "touch",
      button: 0,
      buttons: 1,
    })
  );
  target.dispatchEvent(
    new PointerEventCtor("pointerup", {
      clientX: x,
      clientY: y,
      pointerId,
      pointerType: "touch",
      button: 0,
      buttons: 0,
    })
  );
}

test("zoomAroundPoint keeps focal point stationary while scaling", () => {
  const viewport = { width: 1000, height: 800 };
  const content = { width: 1000, height: 800 };
  const focal = { x: 200, y: -100 };

  const zoomed = zoomAroundPoint(
    { scale: 1, offset: { x: 0, y: 0 } },
    2,
    focal,
    focal,
    viewport,
    content
  );

  assert.equal(zoomed.scale, 2);
  assert.deepEqual(zoomed.offset, { x: -200, y: 100 });
});

test("zoomAroundPoint supports simultaneous pinch scale and midpoint pan", () => {
  const viewport = { width: 1000, height: 800 };
  const content = { width: 1000, height: 800 };

  const next = zoomAroundPoint(
    { scale: 2, offset: { x: 0, y: 0 } },
    3,
    { x: 100, y: 0 },
    { x: 130, y: 20 },
    viewport,
    content
  );

  assert.equal(next.scale, 3);
  assert.deepEqual(next.offset, { x: -20, y: 20 });
});

test("zoomAroundPoint clamps scale to [MIN_IMAGE_SCALE, MAX_IMAGE_SCALE] and resets offset at 1x", () => {
  const viewport = { width: 800, height: 600 };
  const zoomedOut = zoomAroundPoint(
    { scale: 2, offset: { x: 120, y: -80 } },
    0.4,
    { x: 50, y: 50 },
    { x: 50, y: 50 },
    viewport
  );
  assert.equal(zoomedOut.scale, MIN_IMAGE_SCALE);
  assert.deepEqual(zoomedOut.offset, { x: 0, y: 0 });

  const zoomedMax = zoomAroundPoint(
    { scale: 4, offset: { x: 0, y: 0 } },
    9,
    { x: 0, y: 0 },
    { x: 0, y: 0 },
    viewport
  );
  assert.equal(zoomedMax.scale, MAX_IMAGE_SCALE);
});

test("toggleDoubleTapZoom zooms in around tap point and resets back to 1x", () => {
  const viewport = { width: 1000, height: 800 };
  const content = { width: 1000, height: 800 };

  const zoomedIn = toggleDoubleTapZoom(
    { scale: 1, offset: { x: 0, y: 0 } },
    { x: 100, y: -50 },
    viewport,
    content
  );
  assert.equal(zoomedIn.scale, DOUBLE_TAP_SCALE);
  assert.ok(zoomedIn.offset.x < 0);
  assert.ok(zoomedIn.offset.y > 0);

  const reset = toggleDoubleTapZoom(zoomedIn, { x: 0, y: 0 }, viewport, content);
  assert.equal(reset.scale, 1);
  assert.deepEqual(reset.offset, { x: 0, y: 0 });
});

test("panByDelta only pans when scale > 1 and clamps to image bounds", () => {
  const viewport = { width: 1000, height: 800 };
  const content = { width: 600, height: 800 };

  const atOne = panByDelta(
    { scale: 1, offset: { x: 0, y: 0 } },
    { x: 50, y: 50 },
    viewport,
    content
  );
  assert.deepEqual(atOne.offset, { x: 0, y: 0 });

  const panned = panByDelta(
    { scale: 2, offset: { x: 0, y: 0 } },
    { x: 250, y: -500 },
    viewport,
    content
  );
  assert.deepEqual(panned.offset, { x: 100, y: -400 });

  const clampedNarrow = clampOffset(
    { x: 80, y: 80 },
    1.5,
    viewport,
    { width: 500, height: 800 }
  );
  assert.equal(clampedNarrow.x, 0);
  assert.equal(clampedNarrow.y, 80);
});

test("getPinchMetrics and isDoubleTapCandidate compute distance, midpoint, and tap window", () => {
  const metrics = getPinchMetrics({ x: 10, y: 20 }, { x: 13, y: 24 });
  assert.equal(metrics.distance, 5);
  assert.deepEqual(metrics.midpoint, { x: 11.5, y: 22 });

  assert.equal(
    isDoubleTapCandidate(
      { time: 1000, point: { x: 100, y: 100 } },
      { time: 1000 + DOUBLE_TAP_DELAY_MS - 10, point: { x: 110, y: 105 } }
    ),
    true
  );
  assert.equal(
    isDoubleTapCandidate(
      { time: 1000, point: { x: 100, y: 100 } },
      { time: 1000 + DOUBLE_TAP_DELAY_MS + 50, point: { x: 100, y: 100 } }
    ),
    false
  );
  assert.equal(
    isDoubleTapCandidate(
      { time: 1000, point: { x: 100, y: 100 } },
      { time: 1100, point: { x: 200, y: 200 } }
    ),
    false
  );
});

test("FilePreviewModal behavioral: two quick distant taps restore overlay visibility without zooming, while two close taps zoom without toggling overlay", async (t) => {
  const FilePreviewModal = await loadFilePreviewModalComponent();
  const env = setupDomEnvironment();
  const rootEl = document.getElementById("root")!;
  const root = createRoot(rootEl);
  t.after(() => {
    act(() => {
      root.unmount();
    });
    env.cleanup();
  });

  const preview = {
    kind: "image" as const,
    fileName: "photo.png",
    mimeType: "image/png",
    sizeBytes: 4096,
    url: "http://127.0.0.1:12345/preview/tok",
    textContent: null,
    truncated: false,
    sourcePath: "/tmp/photo.png",
  };

  await act(async () => {
    root.render(
      React.createElement(FilePreviewModal, {
        t: translations.ru,
        preview,
        loading: false,
        onClose: () => {},
        onError: () => {},
        onNotice: () => {},
      })
    );
  });

  // Rendered via React Portal into document.body (outside #root)
  assert.equal(rootEl.children.length, 0);
  const dialog = document.body.querySelector('[role="dialog"]') as HTMLElement;
  assert.ok(dialog, "dialog must be portaled into document.body");
  assert.match(dialog.className, /\bfixed\b/);
  assert.match(dialog.className, /\binset-0\b/);
  assert.match(dialog.className, /\bw-screen\b/);
  assert.match(dialog.className, /\bh-screen\b/);

  // No zoom buttons in control bar
  const overlayBar = dialog.firstElementChild as HTMLElement;
  assert.equal(overlayBar.getAttribute("aria-hidden"), "false");
  assert.equal(overlayBar.querySelectorAll("button").length, 2); // Open externally + Close

  const img = dialog.querySelector("img") as HTMLImageElement;
  const viewport = img.parentElement as HTMLElement;
  viewport.getBoundingClientRect = () =>
    ({
      left: 0,
      top: 0,
      width: 1000,
      height: 800,
      right: 1000,
      bottom: 800,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;

  // 1) Two quick taps (< 260ms) at distant points: (100, 100) then (500, 500)
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 100, 100, 1);
  });
  await new Promise((r) => setTimeout(r, 80));
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 500, 500, 2);
  });

  // Wait past DOUBLE_TAP_DELAY_MS so both tap toggles settle
  await act(async () => {
    await new Promise((r) => setTimeout(r, DOUBLE_TAP_DELAY_MS + 60));
  });

  assert.equal(
    overlayBar.getAttribute("aria-hidden"),
    "false",
    "two distant quick taps must toggle overlay twice and return it to visible"
  );
  assert.equal(
    parseScaleFromTransform(img.style.transform),
    1,
    "two distant quick taps must not zoom the image"
  );

  // 2) Single tap hides overlay
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 250, 250, 3);
    await new Promise((r) => setTimeout(r, DOUBLE_TAP_DELAY_MS + 60));
  });
  assert.equal(overlayBar.getAttribute("aria-hidden"), "true");

  // 3) Two close taps (< 260ms, <= 28px) zoom in without toggling overlay back on
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 300, 300, 4);
  });
  await new Promise((r) => setTimeout(r, 80));
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 306, 304, 5);
    await new Promise((r) => setTimeout(r, DOUBLE_TAP_DELAY_MS + 60));
  });

  assert.equal(
    parseScaleFromTransform(img.style.transform),
    DOUBLE_TAP_SCALE,
    "two close quick taps must zoom image to DOUBLE_TAP_SCALE"
  );
  assert.equal(
    overlayBar.getAttribute("aria-hidden"),
    "true",
    "double-tap zoom must not toggle overlay visibility"
  );

  // 4) Two close taps again reset zoom back to 1x without toggling overlay
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 300, 300, 6);
  });
  await new Promise((r) => setTimeout(r, 80));
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 302, 301, 7);
    await new Promise((r) => setTimeout(r, DOUBLE_TAP_DELAY_MS + 60));
  });

  assert.equal(parseScaleFromTransform(img.style.transform), 1);
  assert.equal(overlayBar.getAttribute("aria-hidden"), "true");

  // 5) Single tap shows overlay again
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 250, 250, 8);
    await new Promise((r) => setTimeout(r, DOUBLE_TAP_DELAY_MS + 60));
  });
  assert.equal(overlayBar.getAttribute("aria-hidden"), "false");
});

test("FilePreviewModal behavioral: pinch-to-zoom, post-pinch pan, and wheel zoom after loading transition", async (t) => {
  const FilePreviewModal = await loadFilePreviewModalComponent();
  const env = setupDomEnvironment();
  const rootEl = document.getElementById("root")!;
  const root = createRoot(rootEl);
  t.after(() => {
    act(() => {
      root.unmount();
    });
    env.cleanup();
  });

  const preview = {
    kind: "image" as const,
    fileName: "photo.png",
    mimeType: "image/png",
    sizeBytes: 4096,
    url: "http://127.0.0.1:12345/preview/tok2",
    textContent: null,
    truncated: false,
    sourcePath: "/tmp/photo2.png",
  };

  // Start with loading=true and preview already populated, then flip loading=false
  await act(async () => {
    root.render(
      React.createElement(FilePreviewModal, {
        t: translations.ru,
        preview,
        loading: true,
        onClose: () => {},
        onError: () => {},
        onNotice: () => {},
      })
    );
  });

  await act(async () => {
    root.render(
      React.createElement(FilePreviewModal, {
        t: translations.ru,
        preview,
        loading: false,
        onClose: () => {},
        onError: () => {},
        onNotice: () => {},
      })
    );
  });

  const dialog = document.body.querySelector('[role="dialog"]') as HTMLElement;
  const overlayBar = dialog.firstElementChild as HTMLElement;
  const img = dialog.querySelector("img") as HTMLImageElement;
  const viewport = img.parentElement as HTMLElement;
  viewport.getBoundingClientRect = () =>
    ({
      left: 0,
      top: 0,
      width: 1000,
      height: 800,
      right: 1000,
      bottom: 800,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;

  // Wheel zoom works after loading=true -> false
  await act(async () => {
    viewport.dispatchEvent(
      new env.dom.window.WheelEvent("wheel", {
        deltaY: -200,
        clientX: 500,
        clientY: 400,
        bubbles: true,
        cancelable: true,
      })
    );
  });
  const scaleAfterWheel = parseScaleFromTransform(img.style.transform);
  assert.ok(scaleAfterWheel > 1, "wheel up must zoom in after loading flips false");

  // Two-finger pinch-to-zoom further in, then lift finger 2 and pan with finger 1
  await act(async () => {
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointerdown", {
        clientX: 450,
        clientY: 400,
        pointerId: 10,
        pointerType: "touch",
      })
    );
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointerdown", {
        clientX: 550,
        clientY: 400,
        pointerId: 11,
        pointerType: "touch",
      })
    );
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointermove", {
        clientX: 400,
        clientY: 400,
        pointerId: 10,
        pointerType: "touch",
      })
    );
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointermove", {
        clientX: 600,
        clientY: 400,
        pointerId: 11,
        pointerType: "touch",
      })
    );
  });

  const scaleAfterPinch = parseScaleFromTransform(img.style.transform);
  assert.ok(
    scaleAfterPinch > scaleAfterWheel,
    "spreading two fingers must increase scale"
  );
  const offsetAfterPinch = parseTranslateFromTransform(img.style.transform);

  // Lift finger 2, pan with finger 1, then lift finger 1
  await act(async () => {
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointerup", {
        clientX: 600,
        clientY: 400,
        pointerId: 11,
        pointerType: "touch",
      })
    );
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointermove", {
        clientX: 440,
        clientY: 430,
        pointerId: 10,
        pointerType: "touch",
      })
    );
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointerup", {
        clientX: 440,
        clientY: 430,
        pointerId: 10,
        pointerType: "touch",
      })
    );
    await new Promise((r) => setTimeout(r, DOUBLE_TAP_DELAY_MS + 60));
  });

  const offsetAfterPan = parseTranslateFromTransform(img.style.transform);
  assert.equal(
    offsetAfterPan.x - offsetAfterPinch.x,
    40,
    "remaining finger movement after pinch must pan horizontally by +40px"
  );
  assert.equal(
    offsetAfterPan.y - offsetAfterPinch.y,
    30,
    "remaining finger movement after pinch must pan vertically by +30px"
  );
  assert.equal(
    overlayBar.getAttribute("aria-hidden"),
    "false",
    "pinch + pan must not toggle overlay visibility"
  );
});

test("FilePreviewModal behavioral: tap followed immediately by drag from the same point at scale > 1 toggles overlay once, keeps scale, and pans image", async (t) => {
  const FilePreviewModal = await loadFilePreviewModalComponent();
  const env = setupDomEnvironment();
  const rootEl = document.getElementById("root")!;
  const root = createRoot(rootEl);
  t.after(() => {
    act(() => {
      root.unmount();
    });
    env.cleanup();
  });

  const preview = {
    kind: "image" as const,
    fileName: "photo.png",
    mimeType: "image/png",
    sizeBytes: 4096,
    url: "http://127.0.0.1:12345/preview/tok3",
    textContent: null,
    truncated: false,
    sourcePath: "/tmp/photo3.png",
  };

  await act(async () => {
    root.render(
      React.createElement(FilePreviewModal, {
        t: translations.ru,
        preview,
        loading: false,
        onClose: () => {},
        onError: () => {},
        onNotice: () => {},
      })
    );
  });

  const dialog = document.body.querySelector('[role="dialog"]') as HTMLElement;
  const overlayBar = dialog.firstElementChild as HTMLElement;
  const img = dialog.querySelector("img") as HTMLImageElement;
  const viewport = img.parentElement as HTMLElement;
  viewport.getBoundingClientRect = () =>
    ({
      left: 0,
      top: 0,
      width: 1000,
      height: 800,
      right: 1000,
      bottom: 800,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;

  // Zoom in first via wheel so scale > 1 while overlay remains visible
  await act(async () => {
    viewport.dispatchEvent(
      new env.dom.window.WheelEvent("wheel", {
        deltaY: -300,
        clientX: 500,
        clientY: 400,
        bubbles: true,
        cancelable: true,
      })
    );
  });
  const initialScale = parseScaleFromTransform(img.style.transform);
  assert.ok(initialScale > 1, "precondition: scale must be > 1");
  assert.equal(overlayBar.getAttribute("aria-hidden"), "false");
  const offsetBefore = parseTranslateFromTransform(img.style.transform);

  // Tap at (500, 400), then within < 260ms start dragging from the SAME point (500, 400)
  await act(async () => {
    dispatchTap(viewport, env.PointerEventCtor, 500, 400, 20);
  });
  await new Promise((r) => setTimeout(r, 60));
  await act(async () => {
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointerdown", {
        clientX: 500,
        clientY: 400,
        pointerId: 21,
        pointerType: "touch",
      })
    );
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointermove", {
        clientX: 560,
        clientY: 440,
        pointerId: 21,
        pointerType: "touch",
      })
    );
    viewport.dispatchEvent(
      new env.PointerEventCtor("pointerup", {
        clientX: 560,
        clientY: 440,
        pointerId: 21,
        pointerType: "touch",
      })
    );
    await new Promise((r) => setTimeout(r, DOUBLE_TAP_DELAY_MS + 60));
  });

  assert.equal(
    overlayBar.getAttribute("aria-hidden"),
    "true",
    "first tap must toggle overlay once even when followed immediately by drag from same point"
  );
  assert.equal(
    parseScaleFromTransform(img.style.transform),
    initialScale,
    "drag from same point must not trigger double-tap zoom"
  );
  const offsetAfter = parseTranslateFromTransform(img.style.transform);
  assert.equal(offsetAfter.x - offsetBefore.x, 60);
  assert.equal(offsetAfter.y - offsetBefore.y, 40);
});

test("App toast layer sits above FilePreviewModal portal", () => {
  const appSrc = fs.readFileSync(path.join(rootDir, "src/App.tsx"), "utf8");
  assert.match(appSrc, /z-\[60\]/);
  assert.ok(pathToFileURL(rootDir).href.length > 0);
});
