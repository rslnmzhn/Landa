export const MIN_IMAGE_SCALE = 1;
export const MAX_IMAGE_SCALE = 5;
export const DOUBLE_TAP_SCALE = 2.5;
export const DOUBLE_TAP_DELAY_MS = 260;
export const DOUBLE_TAP_MAX_DISTANCE_PX = 28;
export const TAP_SLOP_PX = 8;

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface TransformState {
  scale: number;
  offset: Point;
}

export interface TapRecord {
  time: number;
  point: Point;
}

export function clampScale(scale: number): number {
  if (!Number.isFinite(scale)) return MIN_IMAGE_SCALE;
  return Math.min(MAX_IMAGE_SCALE, Math.max(MIN_IMAGE_SCALE, scale));
}

export function clampOffset(
  offset: Point,
  scale: number,
  viewport: Size,
  content?: Size
): Point {
  if (scale <= MIN_IMAGE_SCALE || viewport.width <= 0 || viewport.height <= 0) {
    return { x: 0, y: 0 };
  }

  const baseW = content && content.width > 0 ? content.width : viewport.width;
  const baseH = content && content.height > 0 ? content.height : viewport.height;

  const maxX = Math.max(0, (baseW * scale - viewport.width) / 2);
  const maxY = Math.max(0, (baseH * scale - viewport.height) / 2);

  return {
    x: Math.min(maxX, Math.max(-maxX, offset.x)),
    y: Math.min(maxY, Math.max(-maxY, offset.y)),
  };
}

export function zoomAroundPoint(
  state: TransformState,
  nextScaleRaw: number,
  prevFocalFromCenter: Point,
  nextFocalFromCenter: Point,
  viewport: Size,
  content?: Size
): TransformState {
  const nextScale = clampScale(nextScaleRaw);
  if (nextScale <= MIN_IMAGE_SCALE) {
    return { scale: MIN_IMAGE_SCALE, offset: { x: 0, y: 0 } };
  }

  const prevScale = state.scale > 0 ? state.scale : MIN_IMAGE_SCALE;
  const ratio = nextScale / prevScale;
  const rawOffset: Point = {
    x: nextFocalFromCenter.x - ratio * (prevFocalFromCenter.x - state.offset.x),
    y: nextFocalFromCenter.y - ratio * (prevFocalFromCenter.y - state.offset.y),
  };

  return {
    scale: nextScale,
    offset: clampOffset(rawOffset, nextScale, viewport, content),
  };
}

export function toggleDoubleTapZoom(
  state: TransformState,
  tapFromCenter: Point,
  viewport: Size,
  content?: Size
): TransformState {
  if (state.scale > MIN_IMAGE_SCALE + 0.05) {
    return { scale: MIN_IMAGE_SCALE, offset: { x: 0, y: 0 } };
  }
  return zoomAroundPoint(
    state,
    DOUBLE_TAP_SCALE,
    tapFromCenter,
    tapFromCenter,
    viewport,
    content
  );
}

export function panByDelta(
  state: TransformState,
  delta: Point,
  viewport: Size,
  content?: Size
): TransformState {
  if (state.scale <= MIN_IMAGE_SCALE) {
    return { scale: MIN_IMAGE_SCALE, offset: { x: 0, y: 0 } };
  }
  return {
    scale: state.scale,
    offset: clampOffset(
      { x: state.offset.x + delta.x, y: state.offset.y + delta.y },
      state.scale,
      viewport,
      content
    ),
  };
}

export function getPinchMetrics(
  p1: Point,
  p2: Point
): { distance: number; midpoint: Point } {
  return {
    distance: Math.hypot(p2.x - p1.x, p2.y - p1.y),
    midpoint: {
      x: (p1.x + p2.x) / 2,
      y: (p1.y + p2.y) / 2,
    },
  };
}

export function isDoubleTapCandidate(
  prevTap: TapRecord | null,
  nextTap: TapRecord
): boolean {
  if (!prevTap) return false;
  const dt = nextTap.time - prevTap.time;
  if (dt < 0 || dt > DOUBLE_TAP_DELAY_MS) return false;
  const dist = Math.hypot(
    nextTap.point.x - prevTap.point.x,
    nextTap.point.y - prevTap.point.y
  );
  return dist <= DOUBLE_TAP_MAX_DISTANCE_PX;
}
