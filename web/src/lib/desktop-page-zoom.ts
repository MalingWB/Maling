import { INTERFACE_ZOOM_STEP } from '@/lib/desktop-zoom';

export function isDesktopPageZoomShortcut(
  key: string,
  ctrlKey: boolean,
  metaKey: boolean,
) {
  return (ctrlKey || metaKey) && ['+', '=', '-', '0'].includes(key);
}

/**
 * Zoom driver installed by the app shell. When present, the native page-zoom
 * gestures (Ctrl+wheel, Ctrl+`+/-/0`) are consumed and forwarded to the app's
 * owned zoom preference instead of the webview's untracked browser zoom.
 */
export interface DesktopPageZoomDriver {
  /** Adjust zoom by a signed step (positive zooms in). */
  adjust(deltaPercent: number): void;
  /** Reset zoom to the default percent. */
  reset(): void;
}

let driver: DesktopPageZoomDriver | null = null;

export function setDesktopPageZoomDriver(next: DesktopPageZoomDriver | null) {
  driver = next;
}

export function installDesktopPageZoomGuard(target: Window = window) {
  const preventPageWheelZoom = (event: WheelEvent) => {
    if (!event.ctrlKey) return;
    // Native webview zoom must never diverge from the persisted preference;
    // always consume the gesture, with or without an installed driver.
    event.preventDefault();
    driver?.adjust(event.deltaY < 0 ? INTERFACE_ZOOM_STEP : -INTERFACE_ZOOM_STEP);
  };
  const preventPageKeyboardZoom = (event: KeyboardEvent) => {
    if (!isDesktopPageZoomShortcut(event.key, event.ctrlKey, event.metaKey)) return;
    event.preventDefault();
    if (event.key === '0') {
      driver?.reset();
      return;
    }
    driver?.adjust(event.key === '-' ? -INTERFACE_ZOOM_STEP : INTERFACE_ZOOM_STEP);
  };

  target.addEventListener('wheel', preventPageWheelZoom, { capture: true, passive: false });
  target.addEventListener('keydown', preventPageKeyboardZoom, { capture: true });
  return () => {
    target.removeEventListener('wheel', preventPageWheelZoom, { capture: true });
    target.removeEventListener('keydown', preventPageKeyboardZoom, { capture: true });
  };
}
