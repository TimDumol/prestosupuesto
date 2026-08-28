import AsyncStorage from '@react-native-async-storage/async-storage';
import type { Account, Category, Transaction } from './domain';

const CACHE_VERSION = 2;
const CACHE_PREFIX = '@presto-presupuesto/finance-v2/';

export type FinanceCache = {
  version: typeof CACHE_VERSION;
  updatedAt: string;
  categories: Category[];
  accounts: Account[];
  transactions: Transaction[];
};

function cacheKey(owner: string) {
  return `${CACHE_PREFIX}${encodeURIComponent(owner.trim().toLowerCase())}`;
}

function isFinanceCache(value: unknown): value is FinanceCache {
  if (!value || typeof value !== 'object') return false;
  const candidate = value as Partial<FinanceCache>;
  return candidate.version === CACHE_VERSION
    && typeof candidate.updatedAt === 'string'
    && Array.isArray(candidate.categories)
    && Array.isArray(candidate.accounts)
    && Array.isArray(candidate.transactions);
}

export async function loadFinanceCache(owner: string) {
  const serialized = await AsyncStorage.getItem(cacheKey(owner));
  if (!serialized) return null;
  try {
    const cached: unknown = JSON.parse(serialized);
    return isFinanceCache(cached) ? cached : null;
  } catch {
    return null;
  }
}

export async function saveFinanceCache(
  owner: string,
  data: Pick<FinanceCache, 'categories' | 'accounts' | 'transactions'>,
) {
  const cached: FinanceCache = {
    version: CACHE_VERSION,
    updatedAt: new Date().toISOString(),
    categories: data.categories,
    accounts: data.accounts,
    transactions: data.transactions.slice(0, 200),
  };
  await AsyncStorage.setItem(cacheKey(owner), JSON.stringify(cached));
  return cached.updatedAt;
}

export function clearFinanceCache(owner: string) {
  return AsyncStorage.removeItem(cacheKey(owner));
}
