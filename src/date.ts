const FALLBACK_TIME_ZONE = 'Europe/Madrid';

export function deviceTimeZone() {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || FALLBACK_TIME_ZONE;
  } catch {
    return FALLBACK_TIME_ZONE;
  }
}

export function localDateIso(date = new Date()) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

export function dateFromIso(value: string) {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return new Date();
  return new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]), 12);
}

export function shiftLocalDate(date: Date, days: number) {
  const shifted = new Date(date.getFullYear(), date.getMonth(), date.getDate() + days, 12);
  return localDateIso(shifted);
}

export function relativeDateIso(daysAgo: number, now = new Date()) {
  return shiftLocalDate(now, -daysAgo);
}

export function formatActualDate(value: string) {
  return dateFromIso(value).toLocaleDateString(undefined, {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
  });
}

export function formatTransactionDate(value: string, now = new Date()) {
  const today = localDateIso(now);
  const yesterday = shiftLocalDate(now, -1);
  const actual = formatActualDate(value);
  if (value === today) return `Today · ${actual}`;
  if (value === yesterday) return `Yesterday · ${actual}`;
  return actual;
}

