import { getRuntimeApi } from '@/api/client';
import type { PreferencesVm } from '@/types';

export const INTERFACE_ZOOM_MIN = 70;
export const INTERFACE_ZOOM_MAX = 200;
export const INTERFACE_ZOOM_DEFAULT = 100;
/** Step used by the settings slider and the Ctrl+wheel / Ctrl+`+/-/0` shortcuts. */
export const INTERFACE_ZOOM_STEP = 5;

export function normalizeInterfaceZoomPercent(value: number): number {
  if (!Number.isFinite(value)) return INTERFACE_ZOOM_DEFAULT;
  const stepped = Math.round(value / INTERFACE_ZOOM_STEP) * INTERFACE_ZOOM_STEP;
  return Math.min(INTERFACE_ZOOM_MAX, Math.max(INTERFACE_ZOOM_MIN, stepped));
}

/**
 * Applies a zoom percent to the desktop webview without persisting it.
 * Persistence always flows through `saveDesktopPreferences`; this preview path
 * is shared by the settings slider and the zoom shortcuts.
 */
export async function previewInterfaceZoomPercent(percent: number): Promise<number> {
  return getRuntimeApi().applyDesktopZoom(normalizeInterfaceZoomPercent(percent));
}

export function withInterfaceZoomPercent(
  personalization: PreferencesVm['personalization'],
  percent: number,
): PreferencesVm['personalization'] {
  return { ...personalization, interfaceZoomPercent: normalizeInterfaceZoomPercent(percent) };
}
