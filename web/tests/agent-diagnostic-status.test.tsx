/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('../src/api', () => ({
  createAgent: vi.fn(),
  deleteAgent: vi.fn(),
  doctorAgent: vi.fn(),
  getAgentBindingUsage: vi.fn(),
  previewAgentCacheRepair: vi.fn(),
  repairAgentCache: vi.fn(),
  updateAgent: vi.fn(),
}));
import i18n from '../src/i18n';
import { DiagnosticBadge } from '../src/pages/AgentManagementPage';
vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
let root: Root;
let container: HTMLDivElement;
beforeEach(async () => {
  await i18n.changeLanguage('zh-CN');
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});
it('shows a stale diagnostic as needing recheck instead of normal', async () => {
  await act(async () => root.render(<DiagnosticBadge diagnostic={{ status: 'stale', available: false, checkedAt: '2026-11-12T00:00:00Z' }} />));
  expect(container.textContent).toContain('需重新诊断');
  expect(container.textContent).not.toContain('正常');
});
