import { ByteBuffer, type Definition, type Schema } from '@open-pencil/kiwi/schema-runtime';

/** Interpret the embedded schema with bounded allocations, rather than
 * compiling clipboard-controlled code or trusting array lengths in the data. */
export function readFigmaMessage(schema: Schema, data: Uint8Array): unknown {
  const buffer = new ByteBuffer(data);
  const definitions = new Map(schema.definitions.map(definition => [definition.name, definition]));
  let budget = 1_000_000;
  const requireBytes = (count: number) => {
    if (!Number.isSafeInteger(count) || count < 0 || buffer.offset + count > data.length) throw new Error('Incomplete Figma layer data.');
  };
  const uint = () => {
    let count = 0;
    while (true) {
      requireBytes(count + 1);
      if (!(data[buffer.offset + count++]! & 128)) break;
      if (count >= 5) throw new Error('Invalid Figma integer.');
    }
    return buffer.readVarUint();
  };
  const read = (type: string, depth: number): any => {
    if (--budget < 0 || depth > 100) throw new Error('This Figma selection is too complex. Copy fewer layers.');
    switch (type) {
      case 'bool': case 'byte': requireBytes(1); return type === 'bool' ? !!buffer.readByte() : buffer.readByte();
      case 'uint': return uint();
      case 'int': { const value = uint(); return value & 1 ? ~(value >>> 1) : value >>> 1; }
      case 'float': requireBytes(data[buffer.offset] === 0 ? 1 : 4); return buffer.readVarFloat();
      case 'string': {
        const end = data.indexOf(0, buffer.offset);
        if (end < 0) throw new Error('Incomplete Figma text.');
        const text = new TextDecoder().decode(data.subarray(buffer.offset, end)); buffer.offset = end + 1; return text;
      }
      case 'uint64': case 'int64': {
        let length = 0;
        while (length < 8 && (data[buffer.offset + length]! & 128)) length++;
        requireBytes(length + 1);
        return type === 'uint64' ? buffer.readVarUint64() : buffer.readVarInt64();
      }
    }
    const definition = definitions.get(type);
    if (!definition) throw new Error('Unknown Figma data type.');
    if (definition.kind === 'ENUM') {
      const value = uint(), field = definition.fields.find(field => field.value === value);
      if (!field) throw new Error('Unknown Figma enum value.');
      return field.name;
    }
    const result: Record<string, unknown> = {};
    const fieldValue = (field: Definition['fields'][number]) => {
      if (!field.type) throw new Error('Invalid Figma field.');
      if (!field.isArray) return read(field.type, depth + 1);
      const length = uint();
      if (length > budget || length > data.length - buffer.offset) throw new Error('Invalid Figma array length.');
      if (field.type === 'byte') {
        const bytes = data.slice(buffer.offset, buffer.offset + length); buffer.offset += length; return bytes;
      }
      return Array.from({ length }, () => read(field.type!, depth + 1));
    };
    if (definition.kind === 'STRUCT') {
      for (const field of definition.fields) result[field.name] = fieldValue(field);
    } else {
      const fields = new Map(definition.fields.map(field => [field.value, field]));
      let tag: number;
      while ((tag = uint()) !== 0) {
        if (--budget < 0) throw new Error('This Figma selection is too complex.');
        const field = fields.get(tag);
        if (!field) throw new Error('Unknown Figma field.');
        result[field.name] = fieldValue(field);
      }
    }
    return result;
  };
  const result = read('Message', 0);
  if (buffer.offset !== data.length) throw new Error('Unexpected data after Figma layers.');
  return result;
}
