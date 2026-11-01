import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import {
  INTERFACE_ZOOM_DEFAULT,
  INTERFACE_ZOOM_MAX,
  INTERFACE_ZOOM_MIN,
  normalizeInterfaceZoomPercent,
  withInterfaceZoomPercent,
} from '@/lib/desktop-zoom';

describe('interface zoom preference', () => {
  it('clamps and rounds zoom percents to the configured step', () => {
    expect(normalizeInterfaceZoomPercent(100)).toBe(100);
    expect(normalizeInterfaceZoomPercent(133)).toBe(135);
    expect(normalizeInterfaceZoomPercent(12)).toBe(INTERFACE_ZOOM_MIN);
    expect(normalizeInterfaceZoomPercent(400)).toBe(INTERFACE_ZOOM_MAX);
    expect(normalizeInterfaceZoomPercent(Number.NaN)).toBe(INTERFACE_ZOOM_DEFAULT);
  });

  it('stages the zoom percent into the personalization preference', () => {
    const personalization = withInterfaceZoomPercent({
      schemaVersion: 5,
      typography: {
        ui: { fontStack: { source: 'theme' }, fontSize: { source: 'theme' } },
        editor: { fontStack: { source: 'theme' }, fontSize: { source: 'theme' } },
      },
      wallpaper: { byColorScheme: {} },
      avatars: { agent: { image: { source: 'theme' }, shape: { source: 'theme' } }, user: { image: { source: 'theme' }, shape: { source: 'theme' } } },
      interfaceZoomPercent: 100,
      workflowGraphDirection: 'auto',
    }, 150);
    expect(personalization.interfaceZoomPercent).toBe(150);
    expect(personalization.workflowGraphDirection).toBe('auto');
  });
});

describe('interface zoom wiring contract', () => {
  it('exposes the zoom row in settings with preview and commit handlers', () => {
    const source = readFileSync(path.resolve(__dirname, '../src/pages/SettingsPage.tsx'), 'utf8');
    expect(source).toContain('data-interface-zoom-row="true"');
    expect(source).toContain('previewZoomPercent');
    expect(source).toContain('chooseZoomPercent');
    expect(source).toContain('onValueCommit');
  });

  it('applies the persisted zoom at startup and persists through the shared save path', () => {
    const mainRs = readFileSync(path.resolve(__dirname, '../../src-tauri/src/main.rs'), 'utf8');
    expect(mainRs).toContain('commands::apply_interface_zoom_to_main_window');

    const commandsRs = readFileSync(path.resolve(__dirname, '../../src-tauri/src/commands.rs'), 'utf8');
    expect(commandsRs).toContain('pub fn apply_desktop_zoom');
    // The full preference save path applies the persisted zoom after saving.
    expect(commandsRs.indexOf('apply_interface_zoom_to_main_window(&app_handle, personalization.interface_zoom_percent)')).toBeGreaterThan(0);
  });

  it('installs the shortcut driver bridge in the app shell', () => {
    const app = readFileSync(path.resolve(__dirname, '../src/App.tsx'), 'utf8');
    expect(app).toContain('useDesktopZoomShortcuts(');
  });
});
