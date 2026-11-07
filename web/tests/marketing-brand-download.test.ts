import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import packageJson from '../../package.json';
import { GITHUB, PRODUCT_NAME, inlineDownloadUrl, siteConfig } from '../../marketing/site/config';
import { rebrandSceneEvents } from '../../marketing/site/Replay';

const root = process.cwd();
const siteRoot = resolve('marketing/site');
const demoRoot = resolve('marketing/demo');
const userFacingSources = [
  resolve(siteRoot, 'index.html'),
  resolve(siteRoot, 'main.tsx'),
  resolve(siteRoot, 'content.ts'),
  resolve(demoRoot, 'index.html'),
  resolve(demoRoot, 'DemoApp.tsx'),
  resolve(demoRoot, 'DemoFrame.tsx'),
  resolve(demoRoot, 'api.ts'),
  resolve(demoRoot, 'fixtures.ts'),
  resolve(demoRoot, 'management.ts'),
];

describe('marketing brand and download contract', () => {
  it('uses MALING for user-facing website and demo brand content', () => {
    const main = readFileSync(resolve(siteRoot, 'main.tsx'), 'utf8');
    expect(main).toContain('<DownloadMenu');
    expect(main).not.toContain('Gold Band');
    for (const file of userFacingSources) {
      const text = readFileSync(file, 'utf8');
      expect(text, file).not.toMatch(/Gold Band/);
    }
  });

  it('projects recorded product UI as MALING without rewriting historical assets', () => {
    const events = [{ type: 4, data: { href: 'Gold Band', labels: ['Gold Band', 'MALING'] } }];
    const projected = rebrandSceneEvents(events);
    expect(projected[0]?.data).toEqual({ href: 'MALING', labels: ['MALING', 'MALING'] });
    expect(JSON.stringify(events)).toContain('Gold Band');
  });

  it('derives inline download package URLs from the branch version and WB origin', () => {
    expect(packageJson.version).toBe('0.21.0');
    expect(GITHUB).toBe('https://github.com/MalingWB/Maling');
    const baseUrl = 'https://example.invalid/api/file/download/';
    expect(inlineDownloadUrl('windows', packageJson.version, baseUrl)).toBe(`${baseUrl}MALING_0.21.0_x64-setup.exe`);
    expect(inlineDownloadUrl('mac', packageJson.version, baseUrl)).toBe(`${baseUrl}MALING_0.21.0_aarch64.dmg`);
    expect(inlineDownloadUrl('linux', packageJson.version, baseUrl)).toBe(`${baseUrl}MALING_0.21.0_amd64.AppImage`);
  });

  it('keeps the concrete WB download origin out of source and delivery docs', () => {
    const files = [
      resolve(siteRoot, 'config.ts'),
      resolve(siteRoot, 'DownloadMenu.tsx'),
      resolve(root, 'scripts/build-site-bundle.mjs'),
      resolve(root, 'docs/gold-band/开发计划/官网开发计划.md'),
    ];
    for (const file of files) expect(readFileSync(file, 'utf8'), file).not.toMatch(/maling\.weoa\.com/i);
  });
  it('exposes inline and external download sources without desktop/mobile segmentation', () => {
    const config = siteConfig({ VITE_DEMO_URL: '/demo/', GOLD_BAND_WB_BASE_URL: 'https://example.invalid' }, true);
    expect(config.inlineDownloadBaseUrl).toBe('https://example.invalid/api/file/download/');
    expect(config.appVersion).toBe(packageJson.version);
    expect(config.downloadUrl).toBe('https://github.com/MalingWB/Maling/releases');
    const menu = readFileSync(resolve(siteRoot, 'DownloadMenu.tsx'), 'utf8');
    expect(menu).toContain('<DropdownMenu modal={false}>');
    expect(menu).toContain('data-platform-icon');
    expect(menu).toContain('download-option-icon-tile');
    expect(menu).toContain('data-inline-download-option');
    expect(menu).toContain('data-external-download-link');
    expect(menu).not.toContain('data-download-platform-segment');
    expect(menu).toContain("const NEW_TAB_ATTRIBUTES = { target: '_blank', rel: 'noreferrer' } as const;");
    expect((menu.match(/\.\.\.NEW_TAB_ATTRIBUTES/g) ?? []).length).toBe(2);
    expect(menu).not.toMatch(/<a(?![^>]*\{\.\.\.NEW_TAB_ATTRIBUTES)[^>]*href=/);
    const styles = readFileSync(resolve(siteRoot, 'style.css'), 'utf8');
    expect(styles).toMatch(/\.download-menu\s*\{[^}]*background:\s*var\(--background\)/s);
    expect(styles).toMatch(/\.download-menu\s*\{[^}]*border-radius:\s*18px/s);
    expect(styles).toMatch(/\.download-option\s*\{[^}]*background:\s*transparent/s);
    expect(styles).toMatch(/\.download-option-icon-tile\s*\{[^}]*background:\s*var\(--secondary\)/s);
    expect(styles).toMatch(/\.download-option-icon-tile\s*\{[^}]*border-radius:\s*10px/s);
    expect(styles).toMatch(/\.download-option:hover[^{]*\{[^}]*background:\s*var\(--accent\)/s);
  });
});