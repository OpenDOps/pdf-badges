/** GrapesJS canvas pose. `zoom` is the percent `Canvas.setZoom` takes.
 * `coords` and `origin` are in the centered canvas space `setCoords` uses. */
export interface ViewportPose {
  zoom: number;
  coords: { x: number; y: number };
  origin: { x: number; y: number };
}

/** Scale toward the fingers and follow them in the same gesture.
 * `scale` is the pinch ratio since the gesture started (1 leaves the zoom). */
export function viewportAfterPinch(
  start: ViewportPose,
  scale: number,
  origin: { x: number; y: number },
  min: number,
  max: number,
): { zoom: number; coords: { x: number; y: number } } {
  const zoom = Math.min(max, Math.max(min, start.zoom * scale));
  const zoomDelta = start.zoom === 0 ? 1 : zoom / start.zoom;
  return {
    zoom,
    coords: {
      x: start.origin.x - (start.origin.x - start.coords.x) * zoomDelta + (origin.x - start.origin.x),
      y: start.origin.y - (start.origin.y - start.coords.y) * zoomDelta + (origin.y - start.origin.y),
    },
  };
}

/** Move the sheet by a wheel delta. Matches GrapesJS `setCoords(x - deltaX, y - deltaY)`. */
export function viewportAfterPan(coords: { x: number; y: number }, deltaX: number, deltaY: number): { x: number; y: number } {
  return { x: coords.x - deltaX, y: coords.y - deltaY };
}
