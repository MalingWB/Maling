/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const listMock = vi.fn();
const cleanupMock = vi.fn();
vi.mock('../src/api', () => ({
  forgeListSessions: (...args: unknown[]) => listMock(...args),
  forgeCleanupSessions: (...args: unknown[]) => cleanupMock(...args),
}));

import { ForgeSessionHistoryPage } from '@/pages/ForgeSessionHistoryPage';
import '@/i18n';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

/**
 * 铸造历史页(方案 G2/D3)契约:Forge 会话从默认列表移除后,此页是唯一
 * 管理入口——列表按项目读取;清理属删除任务目录的不可逆操作,必须经
 * AlertDialog 确认且确认后才调用清理接口。
 */
const sessions = [
  {
    taskId: 'task-001',
    title: '登录功能铸造',
    origin: 'forge' as const,
    specSummary: { name: '登录功能流水线', intent: '实现登录并验证', stageCount: 3, workflowCount: 1, openQuestionCount: 2 },
    updatedAt: '1790000000',
  },
  {
    taskId: 'task-002',
    title: '试运行:登录功能流水线',
    origin: 'forge-trial' as const,
    specSummary: null,
    updatedAt: '',
  },
];

describe('ForgeSessionHistoryPage', () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
    listMock.mockReset();
    cleanupMock.mockReset();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it('lists forge and trial sessions with spec summaries', async () => {
    listMock.mockResolvedValue(sessions);
    await act(async () => root.render(<ForgeSessionHistoryPage projectId="project-a" />));
    expect(listMock).toHaveBeenCalledWith('project-a');
    expect(host.textContent).toContain('登录功能铸造');
    expect(host.textContent).toContain('铸造');
    expect(host.textContent).toContain('试运行');
    expect(host.textContent).toContain('3 个阶段');
    expect(host.textContent).toContain('尚未产出铸造草稿');
  });

  it('shows the empty state when the project has no forge sessions', async () => {
    listMock.mockResolvedValue([]);
    await act(async () => root.render(<ForgeSessionHistoryPage projectId="project-a" />));
    expect(host.textContent).toContain('当前项目没有铸造会话');
  });

  it('requires confirmation before invoking cleanup', async () => {
    listMock.mockResolvedValue(sessions);
    cleanupMock.mockResolvedValue(['task-001']);
    await act(async () => root.render(<ForgeSessionHistoryPage projectId="project-a" />));

    // 选中第一行,点“清理所选”。
    const checkbox = host.querySelector<HTMLInputElement>('button[role="checkbox"]');
    expect(checkbox).toBeTruthy();
    await act(async () => checkbox?.click());
    const buttons = [...host.querySelectorAll('button')];
    const cleanupSelected = buttons.find((b) => b.textContent?.includes('清理所选'));
    expect(cleanupSelected).toBeTruthy();
    expect(cleanupSelected!.disabled).toBe(false);
    await act(async () => cleanupSelected?.click());

    // 确认对话框出现(Radix portal 挂 body 级);未确认前清理接口未被调用。
    expect(document.body.textContent).toContain('确认清理铸造会话');
    expect(cleanupMock).not.toHaveBeenCalled();

    const confirm = [...document.querySelectorAll('button')].find((b) => b.textContent === '清理' && b.closest('[role="alertdialog"]'));
    expect(confirm).toBeTruthy();
    await act(async () => confirm?.click());
    expect(cleanupMock).toHaveBeenCalledWith('project-a', ['task-001']);
  });
});
