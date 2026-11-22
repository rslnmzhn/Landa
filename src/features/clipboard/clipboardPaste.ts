export const MAX_CLIPBOARD_IMAGE_BYTES = 10 * 1024 * 1024;

export interface PasteableClipboardFile {
  size: number;
  arrayBuffer: () => Promise<ArrayBuffer>;
}

export interface PasteableClipboardItem {
  type: string;
  getAsFile: () => PasteableClipboardFile | null;
}

export interface ClipboardPasteDeps<TItem> {
  pushLocalImageBytes: (bytes: number[]) => Promise<TItem>;
  onImageSaved: (item: TItem) => Promise<void> | void;
  onError: (msg: string) => void;
  tooLargeMessage: string;
}

export async function processClipboardImagePaste<TItem>(
  items: ArrayLike<PasteableClipboardItem> | undefined | null,
  preventDefault: () => void,
  deps: ClipboardPasteDeps<TItem>
): Promise<boolean> {
  if (!items) return false;

  for (let i = 0; i < items.length; i++) {
    const clipItem = items[i];
    if (!clipItem.type.startsWith("image/")) continue;

    const file = clipItem.getAsFile();
    if (!file) continue;

    preventDefault();

    if (file.size > MAX_CLIPBOARD_IMAGE_BYTES) {
      deps.onError(deps.tooLargeMessage);
      return true;
    }

    try {
      const buf = await file.arrayBuffer();
      if (buf.byteLength > MAX_CLIPBOARD_IMAGE_BYTES) {
        deps.onError(deps.tooLargeMessage);
        return true;
      }
      const saved = await deps.pushLocalImageBytes(
        Array.from(new Uint8Array(buf))
      );
      await deps.onImageSaved(saved);
    } catch (err) {
      deps.onError(String(err));
    }
    return true;
  }

  return false;
}
