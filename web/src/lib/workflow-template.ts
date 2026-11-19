import type { WorkflowDsl, WorkflowModelBindings, WorkflowTemplate } from '@/types';
type Translate = (key: string, options?: Record<string, unknown>) => string;

export function workflowTemplateDisplayName(template: WorkflowTemplate, t: Translate): string {
  if (template.id === 'default') return t('taskList.create.defaultFullWorkflow');
  if (template.id === 'default-lightweight') return t('taskList.create.defaultLightweightWorkflow');
  if (template.id === 'wb-development-cicd') return t('memory.workflow');
  return template.name;
}

/// 模板来源 tag 的视觉语义(方案 §7.4,跨页面一致):
/// 铸造 = 主题 accent、导入 = 中性 secondary、手动 = 灰阶 outline;
/// 内置与旧数据(无 source)不显示。
export type WorkflowTemplateSourceTag = {
  source: 'forged' | 'imported' | 'manual';
  className: string;
};

export function workflowTemplateSourceTag(template: WorkflowTemplate): WorkflowTemplateSourceTag | null {
  if (template.isBuiltIn || !template.source) return null;
  switch (template.source) {
    case 'forged':
      return { source: 'forged', className: 'bg-accent text-accent-foreground border-transparent' };
    case 'imported':
      return { source: 'imported', className: 'bg-secondary text-secondary-foreground border-transparent' };
    case 'manual':
      return { source: 'manual', className: 'text-muted-foreground' };
    default:
      return null;
  }
}

export function hasWorkflowDraftChanges(
  workflow: WorkflowDsl | null | undefined,
  baseline: WorkflowDsl | null | undefined,
): boolean {
  return Boolean(workflow && baseline && JSON.stringify(workflow) !== JSON.stringify(baseline));
}

export function shouldShowDefaultWorkflowSaveAsNotice(
  template: WorkflowTemplate | null | undefined,
  workflow: WorkflowDsl | null | undefined,
  baseline: WorkflowDsl | null | undefined,
): boolean {
  return Boolean(template?.isBuiltIn)
    && hasWorkflowDraftChanges(workflow, baseline);
}

export function hasWorkflowBindingDraftChanges(
  bindings: WorkflowModelBindings | null | undefined,
  baseline: WorkflowModelBindings | null | undefined,
): boolean {
  return Boolean(bindings && baseline && JSON.stringify(bindings.bindings) !== JSON.stringify(baseline.bindings));
}

export function restoreBuiltInWorkflowDefinition(
  baseline: WorkflowDsl,
  bindings: WorkflowModelBindings,
): { workflow: WorkflowDsl; modelBindings: WorkflowModelBindings } {
  const workflow = JSON.parse(JSON.stringify(baseline)) as WorkflowDsl;
  const slots = new Set(workflow.nodes.flatMap((node) => node.type === 'worker' && node.executionSlotId ? [node.executionSlotId] : []));
  return {
    workflow,
    modelBindings: {
      ...bindings,
      bindings: bindings.bindings.filter((binding) => slots.has(binding.executionSlotId)),
    },
  };
}

export function createBlankWorkflowDraft(): WorkflowDsl {
  return {
    version: '0.1',
    id: `workflow-${Date.now().toString(36)}`,
    entry: '',
    control: {},
    nodes: [],
    edges: [],
  };
}
