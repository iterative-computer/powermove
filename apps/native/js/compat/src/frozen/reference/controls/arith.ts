/**
 * Evaluate the deliberately tiny arithmetic language accepted by numeric
 * controls. The parser consumes the complete input and never executes source.
 */
export function parseArithmetic(source: string): number {
  let index = 0;
  const finite = (value: number): number => {
    if (!Number.isFinite(value)) throw new RangeError('Non-finite arithmetic');
    return value;
  };

  const whitespace = (): void => {
    while (/\s/.test(source[index] ?? '')) index++;
  };

  const number = (): number => {
    whitespace();
    const match = /^(?:(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?)/i.exec(source.slice(index));
    if (!match) throw new SyntaxError('Expected a number');
    index += match[0].length;
    return finite(Number(match[0]));
  };

  const primary = (): number => {
    whitespace();
    if (source[index] === '(') {
      index++;
      const value = expression();
      whitespace();
      if (source[index] !== ')') throw new SyntaxError('Expected )');
      index++;
      return value;
    }
    return number();
  };

  const unary = (): number => {
    whitespace();
    if (source[index] === '+') { index++; return unary(); }
    if (source[index] === '-') { index++; return -unary(); }
    return primary();
  };

  const product = (): number => {
    let value = unary();
    for (;;) {
      whitespace();
      const operator = source[index];
      if (operator !== '*' && operator !== '/') return value;
      index++;
      const right = unary();
      value = finite(operator === '*' ? value * right : value / right);
    }
  };

  const expression = (): number => {
    let value = product();
    for (;;) {
      whitespace();
      const operator = source[index];
      if (operator !== '+' && operator !== '-') return value;
      index++;
      const right = product();
      value = finite(operator === '+' ? value + right : value - right);
    }
  };

  try {
    whitespace();
    if (index === source.length) return Number.NaN;
    const value = expression();
    whitespace();
    return index === source.length && Number.isFinite(value) ? value : Number.NaN;
  } catch {
    return Number.NaN;
  }
}

