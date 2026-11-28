export function dataUrlToBlobUrl(dataUrl: string, mimeType?: string): string {
  if (
    typeof URL === "undefined" ||
    typeof URL.createObjectURL !== "function" ||
    typeof Blob === "undefined" ||
    !dataUrl.startsWith("data:")
  ) {
    return dataUrl;
  }
  const commaIdx = dataUrl.indexOf(",");
  if (commaIdx < 0) return dataUrl;
  const header = dataUrl.slice(5, commaIdx);
  const payload = dataUrl.slice(commaIdx + 1);
  const resolvedMime =
    mimeType || header.split(";")[0] || "application/octet-stream";
  if (!header.includes(";base64")) {
    return URL.createObjectURL(
      new Blob([decodeURIComponent(payload)], { type: resolvedMime })
    );
  }
  const binary = atob(payload);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return URL.createObjectURL(new Blob([bytes], { type: resolvedMime }));
}

export function revokeBlobUrl(url: string | null | undefined): void {
  if (
    url &&
    url.startsWith("blob:") &&
    typeof URL !== "undefined" &&
    typeof URL.revokeObjectURL === "function"
  ) {
    URL.revokeObjectURL(url);
  }
}
