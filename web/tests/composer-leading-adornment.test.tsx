/** @vitest-environment jsdom */

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ComposerLeadingAdornment } from '@/components/conversation/ComposerLeadingAdornment';
import type { ConversationComposerRemoteBinding } from '@/lib/conversation-composer-draft';
import type { CommittedSlashItem } from '@/lib/slash-command';
import '@/i18n';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

/**
 * 回归测试：multica 远程绑定 chip 与已提交 @角色 标签必须同时可见。
 *
 * 用户在需求管理点 multica 任务「执行」后，composer 预填需求正文并绑定远程任务；
 * 在正文开头追加 `@角色 ` 指定执行角色是合法组合——发送链路同时转发 role snapshot 与
 * draft.remote（start_remote_conversation_run）。原实现的 leading adornment 槽位互斥
 * （slash wins）会让绑定 chip 消失，用户误以为任务解绑。显示层必须并排呈现两者。
 */
const remoteBinding: ConversationComposerRemoteBinding = {
  source: 'multica',
  remoteTaskId: 'task-1',
  workspaceId: 'ws-1',
  title: '修复登录超时',
  attachments: [],
};

const committedRole: CommittedSlashItem = {
  item: { kind: 'role', id: 'pf-dev', name: '开发', description: '开发角色', content: '完整角色定义' },
  prefix: '@开发',
  suffix: ' 修复登录超时',
};

describe('ComposerLeadingAdornment', () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal('ResizeObserver', class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });

  async function renderAdornment(props: Partial<React.ComponentProps<typeof ComposerLeadingAdornment>> = {}) {
    await act(async () => root.render(
      <ComposerLeadingAdornment
        committedSlashCommand={null}
        remoteBinding={null}
        adornmentRef={React.createRef<HTMLSpanElement>()}
        onUnbindRemoteTask={vi.fn()}
        {...props}
      />,
    ));
  }

  it('shows the slash role tag and the remote binding chip together when both are active', async () => {
    const onUnbindRemoteTask = vi.fn();
    await renderAdornment({ committedSlashCommand: committedRole, remoteBinding, onUnbindRemoteTask });

    const slashTag = host.querySelector('[data-slot="slash-command-input-tag"]');
    expect(slashTag).toBeTruthy();
    expect(slashTag?.textContent).toContain('开发');

    expect(host.textContent).toContain('Multica · 修复登录超时');
    const unbindButton = host.querySelector<HTMLButtonElement>('button[aria-label="解除任务绑定"]');
    expect(unbindButton).toBeTruthy();
    await act(async () => unbindButton?.click());
    expect(onUnbindRemoteTask).toHaveBeenCalledTimes(1);
  });

  it('renders only the slash tag when no remote binding exists', async () => {
    await renderAdornment({ committedSlashCommand: committedRole });

    expect(host.querySelector('[data-slot="slash-command-input-tag"]')).toBeTruthy();
    expect(host.textContent).not.toContain('Multica');
  });

  it('renders only the remote binding chip when no slash command is committed', async () => {
    await renderAdornment({ remoteBinding });

    expect(host.querySelector('[data-slot="slash-command-input-tag"]')).toBeNull();
    expect(host.textContent).toContain('Multica · 修复登录超时');
  });

  it('renders no adornment slot at all when neither is active', async () => {
    await renderAdornment();

    expect(host.querySelector('span')).toBeNull();
  });
});
