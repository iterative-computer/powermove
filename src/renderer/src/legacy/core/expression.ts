/**
 * Bounded project-expression parser and interpreter.
 *
 * Expressions are editable project data, so they must not execute JavaScript in
 * the privileged editor document. This deliberately supports Powermove's
 * numeric expression language—not statements, assignment, constructors,
 * prototypes, closures, or arbitrary function calls.
 */

const MAX_SOURCE_CHARS = 2_000;
const MAX_NODES = 160;
const MAX_DEPTH = 24;
const MAX_CALL_ARGS = 12;

type TokenKind = 'number' | 'string' | 'identifier' | 'operator' | 'eof';
type Token = { kind: TokenKind; value: string | number; at: number };

type Node =
  | { kind: 'literal'; value: unknown }
  | { kind: 'identifier'; name: string }
  | { kind: 'unary'; op: string; value: Node }
  | { kind: 'binary'; op: string; left: Node; right: Node }
  | { kind: 'conditional'; test: Node; yes: Node; no: Node }
  | { kind: 'member'; object: Node; property: Node }
  | { kind: 'call'; callee: Node; args: Node[] };

export interface ExpressionContext {
  t: number;
  T: number;
  fps: number;
  value: unknown;
  layer: Record<string, unknown>;
  comp: Record<string, unknown>;
  param: (name: string) => unknown;
  ch: string;
  idx: number;
}

export interface CompiledExpression {
  needsIdx: boolean;
  evaluate(context: ExpressionContext): unknown;
}

const PRECEDENCE: Readonly<Record<string, number>> = {
  '||': 1,
  '&&': 2,
  '==': 3,
  '!=': 3,
  '===': 3,
  '!==': 3,
  '<': 4,
  '<=': 4,
  '>': 4,
  '>=': 4,
  '+': 5,
  '-': 5,
  '*': 6,
  '/': 6,
  '%': 6,
  '**': 7,
};

const FORBIDDEN_MEMBERS = new Set(['__proto__', 'prototype', 'constructor']);
const MATH_FUNCTIONS: Readonly<Record<string, (...values: number[]) => number>> = Object.freeze({
  abs: Math.abs,
  acos: Math.acos,
  asin: Math.asin,
  atan: Math.atan,
  atan2: Math.atan2,
  ceil: Math.ceil,
  cos: Math.cos,
  exp: Math.exp,
  floor: Math.floor,
  log: Math.log,
  max: Math.max,
  min: Math.min,
  pow: Math.pow,
  round: Math.round,
  sign: Math.sign,
  sin: Math.sin,
  sqrt: Math.sqrt,
  tan: Math.tan,
});
const MATH_CONSTANTS: Readonly<Record<string, number>> = Object.freeze({
  E: Math.E,
  LN2: Math.LN2,
  LN10: Math.LN10,
  LOG2E: Math.LOG2E,
  LOG10E: Math.LOG10E,
  PI: Math.PI,
  SQRT1_2: Math.SQRT1_2,
  SQRT2: Math.SQRT2,
});
const MATH_VALUE = Object.freeze({ ...MATH_CONSTANTS, ...MATH_FUNCTIONS });

class Lexer {
  #at = 0;

  constructor(private readonly source: string) {}

  next(): Token {
    while (/\s/.test(this.source[this.#at] ?? '')) this.#at += 1;
    const at = this.#at;
    if (at >= this.source.length) return { kind: 'eof', value: '', at };
    const first = this.source[at]!;

    if (/\d/.test(first) || first === '.' && /\d/.test(this.source[at + 1] ?? '')) {
      const match = this.source.slice(at).match(/^(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/);
      if (!match) throw new SyntaxError(`Invalid number at ${at}`);
      this.#at += match[0].length;
      const value = Number(match[0]);
      if (!Number.isFinite(value)) throw new SyntaxError(`Invalid number at ${at}`);
      return { kind: 'number', value, at };
    }

    if (first === '"' || first === "'") {
      const quote = first;
      let value = '';
      this.#at += 1;
      while (this.#at < this.source.length) {
        const char = this.source[this.#at++]!;
        if (char === quote) return { kind: 'string', value, at };
        if (char === '\n' || char === '\r') throw new SyntaxError(`Unterminated string at ${at}`);
        if (char !== '\\') { value += char; continue; }
        const escaped = this.source[this.#at++];
        if (escaped === undefined) break;
        const replacements: Record<string, string> = {
          n: '\n', r: '\r', t: '\t', b: '\b', f: '\f', v: '\v',
          '\\': '\\', '"': '"', "'": "'",
        };
        value += replacements[escaped] ?? escaped;
      }
      throw new SyntaxError(`Unterminated string at ${at}`);
    }

    if (/[A-Za-z_$]/.test(first)) {
      const match = this.source.slice(at).match(/^[A-Za-z_$][A-Za-z0-9_$]*/)!;
      this.#at += match[0].length;
      return { kind: 'identifier', value: match[0], at };
    }

    for (const operator of ['===', '!==', '**', '&&', '||', '==', '!=', '<=', '>=']) {
      if (this.source.startsWith(operator, at)) {
        this.#at += operator.length;
        return { kind: 'operator', value: operator, at };
      }
    }
    if ('+-*/%<>()[]!,.?:'.includes(first)) {
      this.#at += 1;
      return { kind: 'operator', value: first, at };
    }
    throw new SyntaxError(`Unsupported token at ${at}`);
  }
}

class Parser {
  #token: Token;
  #nodes = 0;

  constructor(private readonly lexer: Lexer) {
    this.#token = lexer.next();
  }

  parse(): Node {
    const value = this.expression(0, 0);
    if (this.#token.kind !== 'eof') throw new SyntaxError(`Unexpected token at ${this.#token.at}`);
    return value;
  }

  private node<T extends Node>(value: T, depth: number): T {
    this.#nodes += 1;
    if (this.#nodes > MAX_NODES || depth > MAX_DEPTH) throw new SyntaxError('Expression is too complex');
    return value;
  }

  private advance(): Token {
    const current = this.#token;
    this.#token = this.lexer.next();
    return current;
  }

  private take(value: string): boolean {
    if (this.#token.value !== value) return false;
    this.advance();
    return true;
  }

  private require(value: string): void {
    if (!this.take(value)) throw new SyntaxError(`Expected ${value} at ${this.#token.at}`);
  }

  private expression(minimum: number, depth: number): Node {
    let left = this.prefix(depth + 1);
    while (typeof this.#token.value === 'string') {
      const op = this.#token.value;
      const precedence = PRECEDENCE[op];
      if (precedence === undefined || precedence < minimum) break;
      this.advance();
      const right = this.expression(precedence + (op === '**' ? 0 : 1), depth + 1);
      left = this.node({ kind: 'binary', op, left, right }, depth);
    }
    if (minimum === 0 && this.take('?')) {
      const yes = this.expression(0, depth + 1);
      this.require(':');
      const no = this.expression(0, depth + 1);
      left = this.node({ kind: 'conditional', test: left, yes, no }, depth);
    }
    return left;
  }

  private prefix(depth: number): Node {
    if (this.#token.value === '+' || this.#token.value === '-' || this.#token.value === '!') {
      const op = String(this.advance().value);
      return this.node({ kind: 'unary', op, value: this.prefix(depth + 1) }, depth);
    }

    let value: Node;
    if (this.#token.kind === 'number' || this.#token.kind === 'string') {
      value = this.node({ kind: 'literal', value: this.advance().value }, depth);
    } else if (this.#token.kind === 'identifier') {
      const name = String(this.advance().value);
      if (name === 'true' || name === 'false' || name === 'null') {
        value = this.node({ kind: 'literal', value: name === 'null' ? null : name === 'true' }, depth);
      } else {
        value = this.node({ kind: 'identifier', name }, depth);
      }
    } else if (this.take('(')) {
      value = this.expression(0, depth + 1);
      this.require(')');
    } else {
      throw new SyntaxError(`Expected a value at ${this.#token.at}`);
    }

    while (true) {
      if (this.take('.')) {
        if (this.#token.kind !== 'identifier') throw new SyntaxError(`Expected a property at ${this.#token.at}`);
        const property = this.node({ kind: 'literal', value: String(this.advance().value) }, depth + 1);
        value = this.node({ kind: 'member', object: value, property }, depth);
      } else if (this.take('[')) {
        const property = this.expression(0, depth + 1);
        this.require(']');
        value = this.node({ kind: 'member', object: value, property }, depth);
      } else if (this.take('(')) {
        const args: Node[] = [];
        if (!this.take(')')) {
          do {
            if (args.length >= MAX_CALL_ARGS) throw new SyntaxError('Expression call has too many arguments');
            args.push(this.expression(0, depth + 1));
          } while (this.take(','));
          this.require(')');
        }
        value = this.node({ kind: 'call', callee: value, args }, depth);
      } else break;
    }
    return value;
  }
}

function clamp(value: number, minimum: number, maximum: number): number {
  return value < minimum ? minimum : value > maximum ? maximum : value;
}

function helper(name: string, args: unknown[], context: ExpressionContext): unknown {
  const numbers = args.map(Number);
  switch (name) {
    case 'clamp': return clamp(numbers[0]!, numbers[1]!, numbers[2]!);
    case 'lerp': return numbers[0]! + (numbers[1]! - numbers[0]!) * numbers[2]!;
    case 'linear': {
      const [x, x0, x1, y0, y1] = numbers;
      return x1 === x0 ? y0 : y0! + (y1! - y0!) * clamp((x! - x0!) / (x1! - x0!), 0, 1);
    }
    case 'ease': {
      const [x, x0, x1, y0, y1] = numbers;
      const amount = clamp(x1 === x0 ? 0 : (x! - x0!) / (x1! - x0!), 0, 1);
      return y0! + (y1! - y0!) * amount * amount * (3 - 2 * amount);
    }
    case 'random': {
      const seed = (numbers[0] ?? 1) * 127.1;
      const value = Math.sin(seed) * 43758.5453;
      return value - Math.floor(value);
    }
    case 'wiggle': {
      const frequency = numbers[0] ?? 0;
      let amplitude = numbers[1] ?? 0;
      const seed = (numbers[2] ?? 0) * 17.3;
      let value = 0;
      let currentFrequency = frequency;
      for (let index = 0; index < 3; index += 1) {
        value += amplitude * (
          Math.sin(context.t * currentFrequency * 6.2831 + seed + index * 2.4)
          + Math.sin(context.t * currentFrequency * 3.94 + seed * 1.7 + index)
        ) * 0.5;
        amplitude *= 0.5;
        currentFrequency *= 2.03;
      }
      return value;
    }
    case 'bounce': {
      let x = clamp(numbers[0] ?? 0, 0, 1);
      if (x < 1 / 2.75) return 7.5625 * x * x;
      if (x < 2 / 2.75) return 7.5625 * (x -= 1.5 / 2.75) * x + 0.75;
      if (x < 2.5 / 2.75) return 7.5625 * (x -= 2.25 / 2.75) * x + 0.9375;
      return 7.5625 * (x -= 2.625 / 2.75) * x + 0.984375;
    }
    case 'loop': return numbers[0]! <= 0 ? numbers[1] : numbers[1]! % numbers[0]!;
    case 'pingpong': {
      const duration = numbers[0] ?? 0;
      if (duration <= 0) return numbers[1];
      const position = numbers[1]! % (duration * 2);
      return position < duration ? position : duration * 2 - position;
    }
    case 'param': return context.param(String(args[0] ?? ''));
    default: return undefined;
  }
}

const HELPERS = new Set(['clamp', 'lerp', 'linear', 'ease', 'random', 'wiggle', 'bounce', 'loop', 'pingpong', 'param']);

function identifier(name: string, context: ExpressionContext): unknown {
  if (name === 'time') return context.T;
  if (name === 'PI') return Math.PI;
  if (name === 'Math') return MATH_VALUE;
  if (name === 't' || name === 'T' || name === 'fps' || name === 'value'
    || name === 'layer' || name === 'comp' || name === 'ch' || name === 'idx') return context[name];
  if (HELPERS.has(name) || Object.hasOwn(MATH_FUNCTIONS, name)) return { callable: name };
  return undefined;
}

function member(object: unknown, property: unknown): unknown {
  const key = typeof property === 'number' ? property : String(property);
  if (typeof key === 'string' && FORBIDDEN_MEMBERS.has(key)) return undefined;
  if (object === MATH_VALUE && typeof key === 'string') return Object.hasOwn(MATH_VALUE, key) ? MATH_VALUE[key] : undefined;
  if (Array.isArray(object)) {
    if (key === 'length') return object.length;
    return Number.isSafeInteger(Number(key)) ? object[Number(key)] : undefined;
  }
  if (typeof object === 'string' && key === 'length') return object.length;
  if (!object || typeof object !== 'object') return undefined;
  return Object.hasOwn(object, key) ? (object as Record<string | number, unknown>)[key] : undefined;
}

function evaluate(node: Node, context: ExpressionContext): unknown {
  switch (node.kind) {
    case 'literal': return node.value;
    case 'identifier': return identifier(node.name, context);
    case 'unary': {
      const value = evaluate(node.value, context);
      if (node.op === '!') return !value;
      if (node.op === '-') return -Number(value);
      return Number(value);
    }
    case 'binary': {
      const left = evaluate(node.left, context);
      if (node.op === '&&') return left && evaluate(node.right, context);
      if (node.op === '||') return left || evaluate(node.right, context);
      const right = evaluate(node.right, context);
      switch (node.op) {
        case '+': return (typeof left === 'string' || typeof right === 'string') ? String(left) + String(right) : Number(left) + Number(right);
        case '-': return Number(left) - Number(right);
        case '*': return Number(left) * Number(right);
        case '/': return Number(left) / Number(right);
        case '%': return Number(left) % Number(right);
        case '**': return Number(left) ** Number(right);
        case '<': return Number(left) < Number(right);
        case '<=': return Number(left) <= Number(right);
        case '>': return Number(left) > Number(right);
        case '>=': return Number(left) >= Number(right);
        case '==': return left == right; // Deliberate expression-language coercion.
        case '!=': return left != right; // Deliberate expression-language coercion.
        case '===': return left === right;
        case '!==': return left !== right;
        default: return undefined;
      }
    }
    case 'conditional': return evaluate(node.test, context) ? evaluate(node.yes, context) : evaluate(node.no, context);
    case 'member': return member(evaluate(node.object, context), evaluate(node.property, context));
    case 'call': {
      const args = node.args.map((arg) => evaluate(arg, context));
      if (node.callee.kind === 'identifier') {
        const name = node.callee.name;
        if (HELPERS.has(name)) return helper(name, args, context);
        const math = MATH_FUNCTIONS[name];
        return math ? math(...args.map(Number)) : undefined;
      }
      if (node.callee.kind === 'member'
        && node.callee.object.kind === 'identifier'
        && node.callee.object.name === 'Math'
        && node.callee.property.kind === 'literal') {
        const math = MATH_FUNCTIONS[String(node.callee.property.value)];
        return math ? math(...args.map(Number)) : undefined;
      }
      return undefined;
    }
  }
}

export function compileExpression(source: unknown): CompiledExpression | null {
  if (typeof source !== 'string' || source.length === 0 || source.length > MAX_SOURCE_CHARS) return null;
  try {
    const tree = new Parser(new Lexer(source)).parse();
    return {
      needsIdx: /\bidx\b/.test(source),
      evaluate: (context) => evaluate(tree, context),
    };
  } catch {
    return null;
  }
}

export const EXPRESSION_NAMES = ['time','t','T','fps','value','layer','comp','ch','idx','PI','Math', ...HELPERS, ...Object.keys(MATH_FUNCTIONS)];

/** Validate identifiers on the parsed tree; strings/member keys are not names. */
export function expressionDiagnostic(source: unknown): string | null {
  if (typeof source !== 'string' || source.length > MAX_SOURCE_CHARS) return `Use an expression of at most ${MAX_SOURCE_CHARS} characters.`;
  try {
    const tree = new Parser(new Lexer(source)).parse();
    const known = new Set(EXPRESSION_NAMES);
    const visit = (node: any): void => {
      if (!node || typeof node !== 'object') return;
      if (node.kind === 'identifier' && !known.has(node.name)) throw new Error(`Unknown name “${node.name}”. Use time, value, or a supported function.`);
      if (node.kind === 'call' && node.callee.kind === 'identifier' && !HELPERS.has(node.callee.name) && !Object.hasOwn(MATH_FUNCTIONS,node.callee.name)) throw new Error(`“${node.callee.name}” is not a supported function.`);
      if (node.kind === 'call' && node.callee.kind === 'member' && !(node.callee.object.kind === 'identifier' && node.callee.object.name === 'Math' && Object.hasOwn(MATH_FUNCTIONS,node.callee.property.value))) throw new Error('Only the documented Math functions can be called.');
      for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (typeof value === 'object') visit(value);
    };
    visit(tree); return null;
  } catch (error) { return error instanceof Error ? error.message : 'Check the expression syntax.'; }
}
