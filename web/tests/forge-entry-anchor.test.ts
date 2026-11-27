import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const appSource = readFileSync(fileURLToPath(new URL('../src/App.tsx', import.meta.url)), 'utf8');

/**
 * 铸造入口的挂靠项目必须在调用时解析（真机回归）：`handleForgeTemplate` 曾在闭包里
 * 捕获启动那一帧的 `defaultProjectId`——应用刚启动、侧边栏尚未加载时它是占位 `'default'`。
 * 于是绑定里的项目与 composer 当前项目不一致，铸造标记在发送时被静默丢弃。
 */
describe('forge entry anchor project', () => {
  it('resolves the anchor project at call time through a ref that mirrors the composer project', () => {
    expect(appSource).toContain('forgeAnchorProjectIdRef.current = draftConversationWorkspaceId ?? effectiveWorkspaceId;');
    expect(appSource).toContain('const anchorProjectId = activeWorkspaceIdRef.current ?? forgeAnchorProjectIdRef.current;');
    // 绑定里的项目与 selectDraftConversationWorkspace 用的是同一个值，二者不可能分叉。
    expect(appSource).toContain('selectDraftConversationWorkspace(anchorProjectId);');
    expect(appSource).toContain('sourceProjectId: anchorProjectId,');
    expect(appSource).not.toContain('sourceProjectId: activeWorkspaceIdRef.current ?? defaultProjectId');
  });
});
