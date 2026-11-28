import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_CLIPBOARD_IMAGE_BYTES,
  processClipboardImagePaste,
} from "./clipboardPaste.ts";

test("rejects pasted image exceeding 10 MiB before calling arrayBuffer or pushLocalImageBytes", async () => {
  let preventDefaultCalled = false;
  let arrayBufferCalled = false;
  let pushCalled = false;
  let savedCalled = false;
  const errors: string[] = [];

  const items = [
    {
      type: "image/png",
      getAsFile: () => ({
        size: MAX_CLIPBOARD_IMAGE_BYTES + 1,
        arrayBuffer: async () => {
          arrayBufferCalled = true;
          return new ArrayBuffer(0);
        },
      }),
    },
  ];

  const handled = await processClipboardImagePaste(
    items,
    () => {
      preventDefaultCalled = true;
    },
    {
      pushLocalImageBytes: async () => {
        pushCalled = true;
        return { id: "unexpected" };
      },
      onImageSaved: () => {
        savedCalled = true;
      },
      onError: (msg) => {
        errors.push(msg);
      },
      tooLargeMessage: "Image exceeds 10 MB limit",
    }
  );

  assert.equal(handled, true);
  assert.equal(preventDefaultCalled, true);
  assert.equal(arrayBufferCalled, false);
  assert.equal(pushCalled, false);
  assert.equal(savedCalled, false);
  assert.deepEqual(errors, ["Image exceeds 10 MB limit"]);
});

test("reads and pushes pasted image within 10 MiB limit", async () => {
  let preventDefaultCalled = false;
  let arrayBufferCalled = false;
  let pushedBytes: Uint8Array | null = null;
  let savedItem: { id: string } | null = null;
  const errors: string[] = [];

  const payload = new Uint8Array([137, 80, 78, 71]);
  const items = [
    {
      type: "image/png",
      getAsFile: () => ({
        size: payload.byteLength,
        arrayBuffer: async () => {
          arrayBufferCalled = true;
          return payload.buffer;
        },
      }),
    },
  ];

  const handled = await processClipboardImagePaste(
    items,
    () => {
      preventDefaultCalled = true;
    },
    {
      pushLocalImageBytes: async (bytes) => {
        pushedBytes = bytes;
        return { id: "img-1" };
      },
      onImageSaved: (item) => {
        savedItem = item;
      },
      onError: (msg) => {
        errors.push(msg);
      },
      tooLargeMessage: "Image exceeds 10 MB limit",
    }
  );

  assert.equal(handled, true);
  assert.equal(preventDefaultCalled, true);
  assert.equal(arrayBufferCalled, true);
  assert.deepEqual(pushedBytes, payload);
  assert.deepEqual(savedItem, { id: "img-1" });
  assert.deepEqual(errors, []);
});

test("ignores non-image clipboard items", async () => {
  let preventDefaultCalled = false;
  const items = [
    {
      type: "text/plain",
      getAsFile: () => null,
    },
  ];

  const handled = await processClipboardImagePaste(
    items,
    () => {
      preventDefaultCalled = true;
    },
    {
      pushLocalImageBytes: async () => ({ id: "unused" }),
      onImageSaved: () => {},
      onError: () => {},
      tooLargeMessage: "Image exceeds 10 MB limit",
    }
  );

  assert.equal(handled, false);
  assert.equal(preventDefaultCalled, false);
});
