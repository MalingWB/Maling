import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { looksLikeJsonSchema } from '@/lib/output-schema';

/**
 * 输出 schema 的形态判定：简化字段表（字段名 → 形状）还是旧版 JSON Schema。
 * 工作流编辑器保存时用它拦截旧版写法；后端 `gold_band::dsl::looks_like_json_schema`
 * 有等价实现，两边用例表一致（见 `tests/dsl_validation.rs`）。
 */
describe('looksLikeJsonSchema', () => {
  it.each([
    ['plain fields', { reason: 'String', result: 'boolean' }],
    ['nested simplified shape', { xx: { yy: [{ zz: 'boolean' }] } }],
    // 回归：字段名可以恰好叫 items / type / required，不能只看键名。
    ['field named items', { items: '数组，每项一条核对结果', complete: 'boolean' }],
    ['field named items with array shape', { items: [{ name: 'string', status: 'string' }], total: 'number' }],
    ['field named type', { type: 'string', level: 'string', message: 'string' }],
    ['field named required', { required: 'boolean', properties: { name: 'string' } }],
    ['field named additionalProperties', { additionalProperties: { key: 'string' } }],
    ['field named type next to items', { type: 'string', items: 'string' }],
  ])('accepts the simplified shape: %s', (_label, schema) => {
    expect(looksLikeJsonSchema(schema)).toBe(false);
  });

  it.each([
    ['object with properties and required', { type: 'object', properties: { result: { type: 'boolean' } }, required: ['result'] }],
    ['array of objects', { type: 'array', items: { type: 'object', properties: { id: { type: 'string' } } } }],
    ['properties without a root type', { properties: { result: { type: 'boolean' } } }],
    ['additionalProperties switch', { type: 'object', additionalProperties: false }],
    ['object with required only', { type: 'object', required: ['result'] }],
    ['union type', { type: ['string', 'null'] }],
    ['$schema declaration', { $schema: 'https://json-schema.org/draft/2020-12/schema', type: 'object' }],
    // 旧版基本类型根节点：整个输出就是一个字符串/布尔值，简化字段表不这样表达。
    ['primitive root', { type: 'boolean' }],
  ])('rejects the legacy JSON Schema: %s', (_label, schema) => {
    expect(looksLikeJsonSchema(schema)).toBe(true);
  });

  it.each([[null], ['string'], [42], [[{ type: 'object' }]], [undefined]])('is false for non-objects (%j)', (value) => {
    expect(looksLikeJsonSchema(value)).toBe(false);
  });

  it('is the single implementation used by the workflow editor', () => {
    const editor = readFileSync(fileURLToPath(new URL('../src/components/WorkflowEditor.tsx', import.meta.url)), 'utf8');
    expect(editor).toContain("import { looksLikeJsonSchema } from '@/lib/output-schema'");
    expect(editor).not.toMatch(/function looksLikeJsonSchema\(/);
  });
});
