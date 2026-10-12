/** Mixed separators use the last separator as decimal; a single separator is decimal.
 * Grouping is accepted only in complete groups of three, never silently stripped.
 */
export function parseLocalizedNumber(value: string): number | null {
  let text = value.trim().replace(/^(?:EUR|USD|PHP|[€$₱])\s*/i, '').replace(/\s*(?:EUR|USD|PHP|[€$₱])$/i, '');
  if (!text) return null;
  let sign = '';
  if (/^[+-]/.test(text)) { sign = text[0]; text = text.slice(1); }
  const dot = text.lastIndexOf('.');
  const comma = text.lastIndexOf(',');
  if (dot >= 0 && comma >= 0) {
    const decimal = dot > comma ? '.' : ',';
    const grouping = decimal === '.' ? ',' : '.';
    const parts = text.split(decimal);
    if (parts.length !== 2 || !/^\d+$/.test(parts[1])) return null;
    const groups = parts[0].split(grouping);
    if (!/^\d{1,3}$/.test(groups[0]) || !groups.slice(1).every(group => /^\d{3}$/.test(group))) return null;
    text = groups.join('') + '.' + parts[1];
  } else {
    const separator = dot >= 0 ? '.' : comma >= 0 ? ',' : null;
    if (separator && text.split(separator).length > 2) {
      const groups = text.split(separator);
      if (!/^\d{1,3}$/.test(groups[0]) || !groups.slice(1).every(group => /^\d{3}$/.test(group))) return null;
      text = groups.join('');
    } else text = text.replace(',', '.');
    if (/\s/.test(text)) {
      if (!/^\d{1,3}(?:[ \u00a0\u202f]\d{3})+(?:\.\d+)?$/.test(text)) return null;
      text = text.replace(/[ \u00a0\u202f]/g, '');
    }
  }
  if (!/^(?:\d+(?:\.\d*)?|\.\d+)$/.test(text)) return null;
  const result = Number(sign + text);
  return Number.isFinite(result) ? result : null;
}

export function parseAmount(value: string): number | null {
  const text = value.trim();
  return text.startsWith('=') ? parseFormula(text.slice(1)) : parseLocalizedNumber(text);
}

function parseFormula(value: string) {
  if (value.length > 256) return null;
  let index = 0;
  const skipWhitespace = () => {
    while (/\s/.test(value[index] ?? '')) index += 1;
  };
  const parseExpression = (): number => {
    let result = parseTerm();
    while (true) {
      skipWhitespace();
      const operator = value[index];
      if (operator !== '+' && operator !== '-') return result;
      index += 1;
      const right = parseTerm();
      result = operator === '+' ? result + right : result - right;
    }
  };
  const parseTerm = (): number => {
    let result = parseUnary();
    while (true) {
      skipWhitespace();
      const operator = value[index];
      if (operator !== '*' && operator !== '/' && operator !== '×' && operator !== '÷') return result;
      index += 1;
      const right = parseUnary();
      if ((operator === '/' || operator === '÷') && right === 0) throw new Error('Division by zero');
      result = operator === '*' || operator === '×' ? result * right : result / right;
    }
  };
  const parseUnary = (): number => {
    skipWhitespace();
    const operator = value[index];
    if (operator === '+' || operator === '-') {
      index += 1;
      const result = parseUnary();
      return operator === '-' ? -result : result;
    }
    return parsePrimary();
  };
  const parsePrimary = (): number => {
    skipWhitespace();
    if (value[index] === '(') {
      index += 1;
      const result = parseExpression();
      skipWhitespace();
      if (value[index] !== ')') throw new Error('Missing closing parenthesis');
      index += 1;
      return result;
    }
    const match = value.slice(index).match(/^(?:\d[\d.,]*|[.,]\d+)/);
    if (!match) throw new Error('Expected a number');
    index += match[0].length;
    const number = parseLocalizedNumber(match[0]);
    if (number === null) throw new Error('Invalid number');
    return number;
  };

  try {
    const result = parseExpression();
    skipWhitespace();
    return index === value.length && Number.isFinite(result) ? result : null;
  } catch {
    return null;
  }
}


/** Money inputs never silently round additional decimal places. */
export function parseMoneyAmount(value: string): number | null {
  const number = parseAmount(value);
  if (number === null || !Number.isSafeInteger(Math.round(number * 100))
    || Math.abs(number * 100 - Math.round(number * 100)) > 1e-7) return null;
  return Math.round(number * 100) / 100;
}
