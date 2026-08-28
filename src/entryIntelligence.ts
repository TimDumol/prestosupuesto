import type { Account, Category, Currency, Transaction } from './domain';

export type ParsedQuickEntry = {
  amount: number;
  currency: Currency;
  accountQuery: string;
  description: string;
};

const CURRENCY_CODES: Record<string, Currency> = {
  EUR: 'EUR',
  PHP: 'PHP',
  USD: 'USD',
  '€': 'EUR',
  '₱': 'PHP',
  '$': 'USD',
};

const ACCOUNT_ALIASES: Record<string, string> = {
  'th gotyme': 'h gotyme',
};

const STOP_WORDS = new Set(['a', 'an', 'and', 'at', 'aug', 'for', 'from', 'in', 'of', 'on', 'the', 'to']);

const CATEGORY_HINTS = [
  { keywords: ['cleaner', 'cleaning', 'housekeeping'], categoryTerms: ['clean', 'home', 'house', 'household'] },
  { keywords: ['grocery', 'groceries', 'market', 'supermarket'], categoryTerms: ['food', 'grocer', 'market'] },
  { keywords: ['cafe', 'coffee', 'dinner', 'lunch', 'restaurant'], categoryTerms: ['dining', 'eating', 'restaurant', 'food'] },
  { keywords: ['bus', 'metro', 'parking', 'taxi', 'train', 'uber'], categoryTerms: ['car', 'transport', 'travel'] },
  { keywords: ['doctor', 'dentist', 'medicine', 'pharmacy'], categoryTerms: ['health', 'medical'] },
  { keywords: ['electric', 'internet', 'phone', 'utility', 'water'], categoryTerms: ['bill', 'home', 'house', 'utility'] },
  { keywords: ['flight', 'hotel', 'holiday', 'trip'], categoryTerms: ['holiday', 'travel', 'vacation'] },
  { keywords: ['salary', 'payroll'], categoryTerms: ['buffer', 'income', 'saving'] },
];

export function normalizeEntryText(value: string) {
  return value
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim();
}

function parseAmount(value: string) {
  let normalized = value.replace(/\s/g, '');
  const comma = normalized.lastIndexOf(',');
  const dot = normalized.lastIndexOf('.');
  if (comma >= 0 && dot >= 0) {
    const decimal = comma > dot ? ',' : '.';
    const grouping = decimal === ',' ? /\./g : /,/g;
    normalized = normalized.replace(grouping, '').replace(decimal, '.');
  } else {
    const separator = comma >= 0 ? ',' : dot >= 0 ? '.' : null;
    if (separator) {
      const pieces = normalized.split(separator);
      const decimalPlaces = pieces.at(-1)?.length ?? 0;
      normalized = decimalPlaces === 2
        ? `${pieces.slice(0, -1).join('')}.${pieces.at(-1)}`
        : pieces.join('');
    }
  }
  const amount = Number(normalized);
  return Number.isFinite(amount) && amount > 0 ? amount : null;
}

export function parseQuickEntry(value: string): ParsedQuickEntry | null {
  const parts = value.split('/').map((part) => part.trim());
  if (parts.length < 3) return null;
  const amountMatch = /^(EUR|PHP|USD|€|₱|\$)\s*([\d.,\s]+)$/i.exec(parts[0]);
  if (!amountMatch) return null;
  const currency = CURRENCY_CODES[amountMatch[1].toUpperCase()] ?? CURRENCY_CODES[amountMatch[1]];
  const amount = parseAmount(amountMatch[2]);
  const accountQuery = parts[1];
  const description = parts.slice(2).join(' / ').trim();
  if (!currency || !amount || !accountQuery || !description) return null;
  return { amount, currency, accountQuery, description };
}

export function resolveAccountQuery(query: string, accounts: Account[]) {
  const normalized = normalizeEntryText(query);
  const canonical = ACCOUNT_ALIASES[normalized] ?? normalized;
  const exact = accounts.find((account) => normalizeEntryText(account.name) === canonical);
  if (exact) return exact;
  const startsWith = accounts.find((account) => normalizeEntryText(account.name).startsWith(canonical));
  if (startsWith) return startsWith;
  return accounts.find((account) => {
    const candidate = normalizeEntryText(account.name);
    return candidate.includes(canonical) || canonical.includes(candidate);
  }) ?? null;
}

function words(value: string) {
  return normalizeEntryText(value).split(' ').filter((word) => word.length > 1 && !STOP_WORDS.has(word));
}

export function suggestCategory(
  description: string,
  categories: Category[],
  transactions: Transaction[],
  excludedTransactionId?: string,
) {
  const normalized = normalizeEntryText(description);
  const descriptionWords = new Set(words(description));
  if (!normalized || !descriptionWords.size) return null;

  const scores = new Map<string, { score: number; reason: string }>();
  const addScore = (categoryId: string, score: number, reason: string) => {
    const previous = scores.get(categoryId);
    if (!previous || score > previous.score) scores.set(categoryId, { score, reason });
  };

  categories.filter((category) => !category.system).forEach((category) => {
    const categoryWords = words(category.name);
    if (categoryWords.some((word) => descriptionWords.has(word))) {
      addScore(category.id, 65, 'description matches the category name');
    }
    CATEGORY_HINTS.forEach((hint) => {
      const hasKeyword = hint.keywords.some((keyword) => normalized.includes(keyword));
      const categoryMatches = hint.categoryTerms.some((term) => normalizeEntryText(category.name).includes(term));
      if (hasKeyword && categoryMatches) addScore(category.id, 75, 'description keyword');
    });
  });

  transactions.forEach((transaction) => {
    if (transaction.id === excludedTransactionId || transaction.kind !== 'expense' || !transaction.categoryFrom) return;
    const historical = normalizeEntryText(transaction.description);
    if (!historical) return;
    if (historical === normalized) {
      addScore(transaction.categoryFrom, 120, 'same description used before');
      return;
    }
    const historicalWords = new Set(words(transaction.description));
    const overlap = [...descriptionWords].filter((word) => historicalWords.has(word)).length;
    const union = new Set([...descriptionWords, ...historicalWords]).size;
    if (overlap >= 2 || (overlap === 1 && union <= 3)) {
      addScore(transaction.categoryFrom, 70 + (overlap / union) * 30, 'similar description used before');
    }
  });

  const best = [...scores.entries()].sort((left, right) => right[1].score - left[1].score)[0];
  if (!best || best[1].score < 60) return null;
  return { categoryId: best[0], reason: best[1].reason };
}

