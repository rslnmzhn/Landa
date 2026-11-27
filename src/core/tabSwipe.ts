export function resolveTabSwipe(
  tabIds: readonly string[],
  activeId: string,
  dx: number,
  dy: number,
  elapsedMs: number
): string | null {
  const absX = Math.abs(dx);
  const absY = Math.abs(dy);
  if (absX <= absY * 1.2) return null;

  const velocity = elapsedMs > 0 ? absX / elapsedMs : 0;
  if (absX < 56 && !(absX >= 36 && velocity > 0.35)) return null;

  const idx = tabIds.indexOf(activeId);
  if (idx < 0) return null;

  const nextIdx = dx < 0 ? idx + 1 : idx - 1;
  return tabIds[nextIdx] ?? null;
}
