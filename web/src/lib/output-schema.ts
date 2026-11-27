/**
 * 输出 schema 的形态判定：简化字段表（字段名 → 形状）还是旧版 JSON Schema。
 *
 * 简化字段表里字符串是类型名、对象是嵌套字段、数组是 `[元素形状]`，字段名可以恰好叫
 * `items` / `type` / `required` / `properties`。所以不能只看键名：只有当关键字位置上的值
 * 具有 JSON Schema 才有的形态时才算 JSON Schema。后端 `gold_band::dsl::looks_like_json_schema`
 * 是等价实现，两边用例表保持一致。
 */

const JSON_SCHEMA_TYPE_NAMES = new Set(['object', 'array', 'string', 'number', 'integer', 'boolean', 'null']);
const DECLARATION_KEYWORDS = ['$schema', '$ref', 'anyOf', 'oneOf', 'allOf'];
const STRUCTURE_KEYWORDS = ['properties', 'required', 'additionalProperties', 'items'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isTypeName(value: unknown): value is string {
  return typeof value === 'string' && JSON_SCHEMA_TYPE_NAMES.has(value);
}

export function looksLikeJsonSchema(schema: unknown): boolean {
  if (!isRecord(schema)) return false;
  const has = (keys: string[]) => keys.some((key) => key in schema);

  // 简化字段表里不会出现的声明式关键字。
  if (has(DECLARATION_KEYWORDS)) return true;
  // additionalProperties 只可能是布尔或子模式；简化形状里不存在布尔值。
  if (typeof schema.additionalProperties === 'boolean') return true;

  const type = schema.type;
  if (Array.isArray(type)) {
    // 联合类型：简化形状的数组只取第一个元素当元素形状，多个类型名的数组只会是 JSON Schema。
    if (type.length > 1 && type.every((entry) => typeof entry === 'string')) return true;
  } else if (isTypeName(type)) {
    // 旧版基本类型根节点：只有 type 一个键，表示整个输出就是一个基本类型值。
    if (Object.keys(schema).length === 1) return true;
    // 容器类型与其它结构关键字同时出现。
    if ((type === 'object' || type === 'array') && has(STRUCTURE_KEYWORDS)) return true;
  }

  // 属性节点：`properties` 的某个值是带 `type` 的子模式（简化形状的嵌套字段不以 type 描述自己）。
  const properties = schema.properties;
  if (isRecord(properties)) {
    const hasTypedProperty = Object.values(properties).some(
      (value) => isRecord(value) && (typeof value.type === 'string' || Array.isArray(value.type)),
    );
    if (hasTypedProperty) return true;
  }
  return false;
}
