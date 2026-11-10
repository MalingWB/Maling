import { readFileSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';
import { getThemePackage, themePackageSummaries } from '../src/theme';

const generatedThemeStyles = () => readFileSync(
  path.resolve(__dirname, '../src/themes/generated/builtin-themes.css'),
  'utf8',
);

describe('builtin.mobius theme package', () => {
  it('ships the localized logo-palette theme with both color schemes', () => {
    const mobius = getThemePackage('builtin.mobius');
    expect(mobius.name).toEqual({ 'zh-CN': '莫比乌斯', en: 'Möbius' });
    expect(mobius.capabilities).toEqual(
      expect.arrayContaining(['tokens', 'component-recipes', 'wallpapers', 'visual-quality-profiles']),
    );
    // Typography inherits the app's canonical font registration instead of shipping font assets.
    expect(mobius.capabilities).not.toContain('fonts');
    expect(mobius.fonts).toBeUndefined();
  });

  it('anchors each scheme on one logo band color', () => {
    const mobius = getThemePackage('builtin.mobius');

    // Dark scheme: near-black logo canvas with the amber band as primary.
    expect(mobius.schemes.dark.semantic).toMatchObject({
      background: '#0f0e10', primary: '#E99D54', primaryForeground: '#221407',
      ring: '#E99D54', link: '#8aa3e8', danger: '#e0564c', popover: '#1a191d',
    });
    // Light scheme: warm paper with the cobalt band as primary and brand vermilion as danger.
    expect(mobius.schemes.light.semantic).toMatchObject({
      background: '#faf8f4', primary: '#244892', primaryForeground: '#ffffff',
      ring: '#244892', link: '#244892', danger: '#CC332B', popover: '#ffffff',
    });
    expect(mobius.schemes.dark.material.model).toBe('solid');
    expect(mobius.schemes.light.material.model).toBe('solid');
  });

  it('declares symmetric wallpapers for both schemes and mutes them in performance mode', () => {
    const mobius = getThemePackage('builtin.mobius');

    for (const scheme of ['light', 'dark'] as const) {
      const schemeWallpapers = mobius.wallpapers?.[scheme] ?? {};
      expect(Object.keys(schemeWallpapers).sort()).toEqual(['conversation', 'settings']);
      for (const descriptor of Object.values(schemeWallpapers)) {
        expect(descriptor.assetId).toBe(`mobius-band-${scheme}`);
        expect(descriptor.fit).toBe('cover');
        expect(descriptor.overlayColor).toBe('background');
      }
    }

    expect(mobius.visualQualityProfiles?.performance.wallpapers).toEqual({ enabled: false });
    expect(generatedThemeStyles()).toContain(
      ":root[data-theme='builtin.mobius'][data-visual-quality='performance']",
    );
  });

  it('registers the wallpaper assets with runtime URLs under the theme directory', () => {
    const mobius = getThemePackage('builtin.mobius');
    const wallpapers = mobius.assets.records.filter((asset) => asset.kind === 'wallpaper');
    expect(wallpapers.map(({ id }) => id).sort()).toEqual(['mobius-band-dark', 'mobius-band-light']);
    for (const asset of wallpapers) {
      expect(asset.outputUrl).toMatch(/^\/theme-assets\/builtin\.mobius\//u);
      expect(asset.width).toBe(2048);
      expect(asset.height).toBe(1280);
    }
  });

  it('keeps the theme selectable from the shared summaries', () => {
    expect(themePackageSummaries.map(({ id }) => id)).toContain('builtin.mobius');
  });
});
