export interface TransferPathItem {
  rawPath: string;
  displayPath: string;
  relativePath: string;
  fileName: string;
}

export interface TransferPathSummary {
  commonDir: string | null;
  items: TransferPathItem[];
}

const URI_SCHEME_RE = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//;
const WINDOWS_PREFIX_RE = /^(?:[A-Za-z]:[\\/]|\\\\)/;

function isUri(value: string): boolean {
  return URI_SCHEME_RE.test(value);
}

function trimTrailingSeparators(value: string): string {
  if (value === "/" || /^[A-Za-z]:[\\/]?$/.test(value)) {
    return value;
  }
  const trimmed = value.replace(/[/\\]+$/, "");
  return trimmed || value;
}

function inferSeparator(value: string): "/" | "\\" {
  if (WINDOWS_PREFIX_RE.test(value)) {
    return "\\";
  }
  if (value.includes("\\") && !value.includes("/")) {
    return "\\";
  }
  return "/";
}

function hasPathSeparator(value: string): boolean {
  return value.includes("/") || value.includes("\\");
}

export function resolveDisplayPath(
  pathOrUri: string,
  resolvedNames: Record<string, string>
): string {
  const resolved = resolvedNames[pathOrUri]?.trim();
  if (resolved && hasPathSeparator(resolved)) {
    return resolved;
  }
  return pathOrUri;
}

export function resolveDisplayName(
  pathOrUri: string,
  resolvedNames: Record<string, string>
): string {
  const resolved = resolvedNames[pathOrUri]?.trim();
  if (resolved) {
    return hasPathSeparator(resolved) ? extractFileName(resolved) : resolved;
  }
  return extractFileName(pathOrUri);
}

export function extractFileName(displayPath: string): string {
  if (isUri(displayPath)) {
    const clean = displayPath.split(/[?#]/)[0].replace(/\/+$/, "");
    const uriParts = clean.split("/").filter(Boolean);
    const rawLast = uriParts[uriParts.length - 1] || displayPath;
    let decoded = rawLast;
    try {
      decoded = decodeURIComponent(rawLast);
    } catch {
      decoded = rawLast;
    }
    const nestedParts = trimTrailingSeparators(decoded)
      .split(/[/\\]/)
      .filter(Boolean);
    return nestedParts[nestedParts.length - 1] || decoded;
  }

  const trimmed = trimTrailingSeparators(displayPath);
  const parts = trimmed.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || displayPath;
}

function normalizeStoredRelativePath(raw: string | undefined): string | null {
  if (!raw) return null;
  const normalized = raw.replace(/\\/g, "/").trim().replace(/^\/+|\/+$/g, "");
  return normalized || null;
}

export function summarizeTransferPaths(
  savedPaths: string[],
  resolvedNames: Record<string, string>,
  relativePaths?: string[]
): TransferPathSummary {
  const hasStoredRelPaths =
    Array.isArray(relativePaths) && relativePaths.length === savedPaths.length;

  const baseItems = savedPaths.map((rawPath, idx) => {
    const displayPath = resolveDisplayPath(rawPath, resolvedNames);
    const fileName = resolveDisplayName(rawPath, resolvedNames);
    const storedRel = hasStoredRelPaths
      ? normalizeStoredRelativePath(relativePaths[idx])
      : null;
    return { rawPath, displayPath, fileName, storedRel };
  });

  if (baseItems.length <= 1) {
    return {
      commonDir: null,
      items: baseItems.map((item) => ({
        rawPath: item.rawPath,
        displayPath: item.displayPath,
        fileName: item.fileName,
        relativePath: item.storedRel || item.fileName,
      })),
    };
  }

  const nonUriEntries = baseItems
    .map((item, index) => {
      if (isUri(item.displayPath)) return null;
      const cleanPath = trimTrailingSeparators(item.displayPath);
      const parts = cleanPath.split(/[/\\]/);
      return {
        index,
        parts,
        dirParts: parts.slice(0, -1),
        sep: inferSeparator(cleanPath),
      };
    })
    .filter((entry): entry is NonNullable<typeof entry> => entry !== null);

  if (nonUriEntries.length < 2) {
    return {
      commonDir: null,
      items: baseItems.map((item) => ({
        rawPath: item.rawPath,
        displayPath: item.displayPath,
        fileName: item.fileName,
        relativePath: item.storedRel || item.fileName,
      })),
    };
  }

  let commonLen = nonUriEntries[0].dirParts.length;
  for (let i = 1; i < nonUriEntries.length; i++) {
    const cur = nonUriEntries[i].dirParts;
    let match = 0;
    const limit = Math.min(commonLen, cur.length);
    while (match < limit && nonUriEntries[0].dirParts[match] === cur[match]) {
      match++;
    }
    commonLen = match;
  }

  const commonSegs = nonUriEntries[0].dirParts.slice(0, commonLen);
  const hasMeaningfulCommonDir =
    commonSegs.length >= 2 ||
    (commonSegs.length === 1 &&
      commonSegs[0] !== "" &&
      !commonSegs[0].endsWith(":"));

  if (!hasMeaningfulCommonDir) {
    return {
      commonDir: null,
      items: baseItems.map((item) => ({
        rawPath: item.rawPath,
        displayPath: item.displayPath,
        fileName: item.fileName,
        relativePath: item.storedRel || item.fileName,
      })),
    };
  }

  const commonDir = commonSegs.join(nonUriEntries[0].sep);
  const nonUriByIndex = new Map(nonUriEntries.map((e) => [e.index, e]));

  return {
    commonDir,
    items: baseItems.map((item, idx) => {
      const parsed = nonUriByIndex.get(idx);
      if (!parsed) {
        return {
          rawPath: item.rawPath,
          displayPath: item.displayPath,
          fileName: item.fileName,
          relativePath: item.storedRel || item.fileName,
        };
      }
      return {
        rawPath: item.rawPath,
        displayPath: item.displayPath,
        fileName: item.fileName,
        relativePath:
          item.storedRel ||
          parsed.parts.slice(commonLen).join("/") ||
          item.fileName,
      };
    }),
  };
}
