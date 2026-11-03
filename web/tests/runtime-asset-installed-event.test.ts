import { describe, expect, it } from 'vitest';
import { RUNTIME_ASSET_INSTALLED_EVENT } from '../src/lib/app-events';
import type { RuntimeAssetInstalledEventVm } from '../src/types';

// M5-cr 被动安装事件契约：事件名与载荷形状是跨端契约（Rust
// `src-tauri/src/multica/runtime_assets.rs` emit ↔ 此处订阅），任一侧改名此处先红。
// Rust 侧载荷经 `#[serde(rename_all = "camelCase")]` 序列化：assetType / installedDisplayName。
describe('runtime-asset-installed event contract', () => {
  it('locks the cross-boundary event name', () => {
    expect(RUNTIME_ASSET_INSTALLED_EVENT).toBe('gold-band://runtime-asset-installed');
  });

  it('payload shape is camelCase with assetType and installedDisplayName', () => {
    const payload: RuntimeAssetInstalledEventVm = {
      assetType: 'workflow',
      installedDisplayName: '我的流水线',
    };
    expect(payload.assetType).toBe('workflow');
    expect(payload.installedDisplayName).toBe('我的流水线');
    expect(Object.keys(payload).sort()).toEqual(['assetType', 'installedDisplayName']);
  });
});
