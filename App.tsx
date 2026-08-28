import DateTimePicker, { DateTimePickerAndroid } from '@react-native-community/datetimepicker';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import {
  initialWindowMetrics,
  SafeAreaProvider,
  SafeAreaView,
} from 'react-native-safe-area-context';
import {
  accounts as initialAccounts,
  Account,
  accountName,
  available,
  Category,
  categoryName,
  Currency,
  descriptionSuggestions,
  initialCategories,
  initialTransactions,
  money,
  SYSTEM_EXPENSE,
  SYSTEM_INCOME,
  Transaction,
  TransactionKind,
} from './src/domain';
import {
  fetchFinanceSnapshot,
  FinanceSnapshot,
  gatewayConfigured,
  restoreGatewayUser,
  signInToGateway,
  signOutFromGateway,
  submitGatewayTransaction,
  updateGatewayTransaction,
} from './src/googleGateway';
import { colors, radius } from './src/theme';
import { dateFromIso, deviceTimeZone, formatActualDate, formatTransactionDate, localDateIso, relativeDateIso } from './src/date';
import { parseQuickEntry, resolveAccountQuery, suggestCategory } from './src/entryIntelligence';
import { clearFinanceCache, loadFinanceCache, saveFinanceCache } from './src/financeCache';
import { mergeTransactions } from './src/transactionCache';
import { uuidV7 } from './src/uuidv7';

type Screen = 'recent' | 'add' | 'budget' | 'setup';
type PickerTarget = 'categoryFrom' | 'categoryTo' | 'accountFrom' | 'accountTo' | 'currency';
type PickerState = { target: PickerTarget; title: string } | null;
type UsageScores = { categories: Record<string, number>; accounts: Record<string, number> };

function normalizedSearch(value: string) {
  return value.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase().trim();
}

function fuzzyMatchScore(label: string, query: string) {
  const candidate = normalizedSearch(label);
  const search = normalizedSearch(query);
  if (!search) return 0;
  if (candidate === search) return 120;
  if (candidate.startsWith(search)) return 100 - candidate.length * 0.01;
  const containedAt = candidate.indexOf(search);
  if (containedAt >= 0) return 80 - containedAt;
  const initials = candidate.split(/[^a-z0-9]+/).filter(Boolean).map((part) => part[0]).join('');
  if (initials.startsWith(search)) return 70;
  let searchIndex = 0;
  let gap = 0;
  let lastMatch = -1;
  for (let index = 0; index < candidate.length && searchIndex < search.length; index += 1) {
    if (candidate[index] === search[searchIndex]) {
      if (lastMatch >= 0) gap += index - lastMatch - 1;
      lastMatch = index;
      searchIndex += 1;
    }
  }
  return searchIndex === search.length ? 50 - gap : null;
}

const kindLabels: Record<TransactionKind, string> = {
  expense: 'Expense',
  income: 'Income',
  transfer: 'Transfer',
  reallocate: 'Reallocate',
};

const typeDefaults: Record<TransactionKind, string> = {
  expense: 'Weekly groceries',
  income: 'Monthly salary',
  transfer: 'Move to savings',
  reallocate: 'Top up home repairs',
};

function Header({ title, connected }: { title: string; connected: boolean }) {
  return (
    <View style={styles.header}>
      <View>
        <Text style={styles.eyebrow}>PRESTO PRESUPUESTO</Text>
        <Text style={styles.headerTitle}>{title}</Text>
      </View>
      <View style={styles.localBadge}>
        <View style={[styles.localDot, connected && styles.connectedDot]} />
        <Text style={styles.localBadgeText}>{connected ? 'Live sheet' : 'Local demo'}</Text>
      </View>
    </View>
  );
}

function FieldLabel({ children }: { children: React.ReactNode }) {
  return <Text style={styles.fieldLabel}>{children}</Text>;
}

function ChoiceCard({
  label,
  value,
  meta,
  icon,
  onPress,
}: {
  label: string;
  value: string;
  meta?: string;
  icon: string;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${label}: ${value}`}
      onPress={onPress}
      style={({ pressed }) => [styles.choiceCard, pressed && styles.pressed]}
    >
      <View style={styles.choiceIcon}><Text style={styles.choiceIconText}>{icon}</Text></View>
      <View style={styles.choiceCopy}>
        <Text style={styles.choiceLabel}>{label}</Text>
        <Text style={styles.choiceValue}>{value}</Text>
        {meta ? <Text style={styles.choiceMeta}>{meta}</Text> : null}
      </View>
      <Text style={styles.changeText}>Change</Text>
    </Pressable>
  );
}

function BudgetStrip({ category }: { category: Category }) {
  const total = Math.max(category.monthly + category.rollover + category.adjustment, 1);
  const remaining = available(category);
  const percentage = Math.max(0, Math.min(100, (remaining / total) * 100));
  return (
    <View style={styles.budgetStrip}>
      <View style={styles.rowBetween}>
        <Text style={styles.availableText}>{money(remaining)} available</Text>
        <Text style={styles.smallMuted}>{money(category.monthly)}/mo + {money(category.rollover + category.adjustment)} rollover</Text>
      </View>
      <View style={styles.track}><View style={[styles.fill, { width: `${percentage}%` }]} /></View>
    </View>
  );
}

function FlowRow({ label, flow }: { label: string; flow: string }) {
  return (
    <View style={styles.flowRow}>
      <Text style={styles.smallMuted}>{label}</Text>
      <Text style={styles.flowValue}>{flow}</Text>
    </View>
  );
}

function TypeTabs({ value, onChange }: { value: TransactionKind; onChange: (kind: TransactionKind) => void }) {
  return (
    <View style={styles.typeTabs} accessibilityRole="tablist">
      {(Object.keys(kindLabels) as TransactionKind[]).map((item) => {
        const active = item === value;
        return (
          <Pressable
            key={item}
            accessibilityRole="tab"
            accessibilityState={{ selected: active }}
            onPress={() => onChange(item)}
            style={({ pressed }) => [styles.typeTab, active && styles.typeTabActive, pressed && styles.pressed]}
          >
            <Text style={[styles.typeTabText, active && styles.typeTabTextActive]}>{kindLabels[item]}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function TransactionRow({ transaction, categories, accounts, onPress }: { transaction: Transaction; categories: Category[]; accounts: Account[]; onPress: () => void }) {
  const isPositive = transaction.kind === 'income';
  const sign = transaction.kind === 'expense' ? '−' : transaction.kind === 'income' ? '+' : '';
  const icon = transaction.kind === 'expense' ? '↘' : transaction.kind === 'income' ? '↗' : transaction.kind === 'transfer' ? '⇄' : '⇆';
  let flow = '';
  if (transaction.kind === 'expense' || transaction.kind === 'income') {
    flow = `${categoryName(categories, transaction.categoryFrom)} → ${categoryName(categories, transaction.categoryTo)}`;
    const account = transaction.accountFrom ?? transaction.accountTo;
    if (account) flow += ` · ${accountName(accounts, account)}`;
  } else if (transaction.kind === 'transfer') {
    flow = `${accountName(accounts, transaction.accountFrom)} → ${accountName(accounts, transaction.accountTo)} · No budget change`;
  } else {
    flow = `${categoryName(categories, transaction.categoryFrom)} → ${categoryName(categories, transaction.categoryTo)} · Budget only`;
  }
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={`Edit ${transaction.description}`} onPress={onPress} style={({ pressed }) => [styles.transactionRow, pressed && styles.pressed]}>
      <View style={styles.transactionIcon}><Text style={styles.transactionIconText}>{icon}</Text></View>
      <View style={styles.transactionCopy}>
        <Text style={styles.transactionTitle}>{transaction.description}</Text>
        <Text style={styles.transactionMeta} numberOfLines={2}>{flow}</Text>
        <Text style={[styles.transactionDate, transaction.syncStatus === 'failed' && styles.syncFailed]}>{transaction.dateLabel}{transaction.syncStatus === 'pending' ? ' · Syncing…' : transaction.syncStatus === 'failed' ? ' · Sync failed' : ''}</Text>
      </View>
      <View style={styles.transactionSide}>
        <Text style={[styles.transactionAmount, isPositive && styles.positive]}>{sign}{money(transaction.amount, transaction.currency)}</Text>
        <Text style={styles.editText}>Edit</Text>
      </View>
    </Pressable>
  );
}

function BudgetRow({ category }: { category: Category }) {
  const usable = available(category);
  const total = Math.max(category.monthly + category.rollover + category.adjustment, 1);
  const percentage = Math.max(0, Math.min(100, (usable / total) * 100));
  return (
    <View style={styles.budgetCard}>
      <View style={styles.rowBetween}>
        <Text style={styles.budgetName}>{category.name}</Text>
        <Text style={[styles.budgetAmount, usable < 0 && styles.negative]}>{money(usable)}</Text>
      </View>
      <Text style={styles.budgetMeta}>{money(category.monthly)} monthly + {money(category.rollover + category.adjustment)} rollover − {money(category.spent)} spent</Text>
      <View style={styles.track}><View style={[styles.fill, usable < 0 && styles.fillDanger, { width: `${percentage}%` }]} /></View>
    </View>
  );
}

function SetupRow({ icon, title, detail, value }: { icon: string; title: string; detail: string; value?: string }) {
  return (
    <View style={styles.setupRow}>
      <View style={styles.setupIcon}><Text>{icon}</Text></View>
      <View style={styles.setupCopy}><Text style={styles.setupTitle}>{title}</Text><Text style={styles.setupDetail}>{detail}</Text></View>
      {value ? <Text style={styles.setupValue}>{value}</Text> : null}
    </View>
  );
}

function BottomNav({ value, onChange }: { value: Screen; onChange: (screen: Screen) => void }) {
  const items: { id: Screen; label: string; icon: string }[] = [
    { id: 'recent', label: 'Recent', icon: '≡' },
    { id: 'add', label: 'Add', icon: '+' },
    { id: 'budget', label: 'Budget', icon: '▥' },
    { id: 'setup', label: 'Setup', icon: '⚙' },
  ];
  return (
    <View style={styles.bottomNav}>
      {items.map((item) => {
        const active = item.id === value;
        return (
          <Pressable
            key={item.id}
            accessibilityRole="tab"
            accessibilityState={{ selected: active }}
            onPress={() => onChange(item.id)}
            style={({ pressed }) => [styles.navItem, active && styles.navItemActive, pressed && styles.pressed]}
          >
            <Text style={[styles.navIcon, active && styles.navTextActive]}>{item.icon}</Text>
            <Text style={[styles.navText, active && styles.navTextActive]}>{item.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function PickerModal({ picker, categories, accounts, usageScores, selected, onSelect, onClose }: {
  picker: PickerState;
  categories: Category[];
  accounts: Account[];
  usageScores: UsageScores;
  selected: string;
  onSelect: (value: string) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState('');
  useEffect(() => setQuery(''), [picker?.target]);

  if (!picker) return null;
  const isCategory = picker.target === 'categoryFrom' || picker.target === 'categoryTo';
  const isAccount = picker.target === 'accountFrom' || picker.target === 'accountTo';
  const baseOptions = isCategory
    ? categories.filter((item) => !item.system).map((item) => ({ id: item.id, label: item.name, meta: `${money(available(item))} available · ${money(item.monthly)}/mo` }))
    : isAccount
      ? accounts.map((item) => ({ id: item.id, label: item.name, meta: item.currency }))
      : (['EUR', 'PHP', 'USD'] as Currency[]).map((item) => ({ id: item, label: item, meta: 'Currency' }));
  const frequency = isCategory ? usageScores.categories : isAccount ? usageScores.accounts : {};
  const options = baseOptions
    .map((item) => ({ ...item, match: fuzzyMatchScore(item.label, query) }))
    .filter((item) => item.match !== null)
    .sort((left, right) => {
      const matchDifference = (right.match ?? 0) - (left.match ?? 0);
      if (query && matchDifference) return matchDifference;
      return (frequency[right.id] ?? 0) - (frequency[left.id] ?? 0) || left.label.localeCompare(right.label);
    });
  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : 'height'} style={styles.modalRoot}>
        <Pressable style={styles.modalBackdrop} onPress={onClose} accessibilityLabel="Close picker" />
        <SafeAreaView edges={['bottom']} style={styles.sheet}>
          <View style={styles.sheetHandle} />
          <View style={styles.sheetHeader}>
            <Text style={styles.sheetTitle}>{picker.title}</Text>
            <Pressable onPress={onClose} style={styles.closeButton}><Text style={styles.closeText}>Close</Text></Pressable>
          </View>
          {isCategory || isAccount ? (
            <View style={styles.searchWrap}>
              <TextInput autoFocus value={query} onChangeText={setQuery} placeholder={`Search ${isCategory ? 'categories' : 'accounts'}…`} placeholderTextColor={colors.muted} returnKeyType="search" style={styles.searchInput} />
            </View>
          ) : null}
          <ScrollView keyboardShouldPersistTaps="handled" style={styles.sheetList} contentContainerStyle={styles.sheetListContent}>
            {options.map((item) => {
              const active = selected === item.id;
              return (
                <Pressable
                  key={item.id}
                  accessibilityRole="button"
                  accessibilityState={{ selected: active }}
                  onPress={() => onSelect(item.id)}
                  style={({ pressed }) => [styles.optionRow, active && styles.optionRowActive, pressed && styles.pressed]}
                >
                  <View style={[styles.radio, active && styles.radioActive]}>{active ? <View style={styles.radioDot} /> : null}</View>
                  <View style={styles.optionCopy}><Text style={styles.optionTitle}>{item.label}</Text><Text style={styles.optionMeta}>{item.meta}</Text></View>
                </Pressable>
              );
            })}
            {!options.length ? <Text style={styles.noResults}>No matching options</Text> : null}
          </ScrollView>
        </SafeAreaView>
      </KeyboardAvoidingView>
    </Modal>
  );
}

export default function App() {
  const [screen, setScreen] = useState<Screen>('add');
  const [kind, setKind] = useState<TransactionKind>('expense');
  const [amount, setAmount] = useState('');
  const [currency, setCurrency] = useState<Currency>('EUR');
  const [description, setDescription] = useState('');
  const [categoryFrom, setCategoryFrom] = useState('food');
  const [categoryTo, setCategoryTo] = useState('home');
  const [accountFrom, setAccountFrom] = useState('visa');
  const [accountTo, setAccountTo] = useState('checking');
  const [date, setDate] = useState(localDateIso());
  const [quickEntry, setQuickEntry] = useState('');
  const [editingTransaction, setEditingTransaction] = useState<Transaction | null>(null);
  const [categoryWasManuallySelected, setCategoryWasManuallySelected] = useState(false);
  const [showIosDatePicker, setShowIosDatePicker] = useState(false);
  const [note, setNote] = useState('');
  const [toAmount, setToAmount] = useState('');
  const [showMore, setShowMore] = useState(false);
  const [picker, setPicker] = useState<PickerState>(null);
  const [categories, setCategories] = useState(initialCategories);
  const [accounts, setAccounts] = useState(initialAccounts);
  const [transactions, setTransactions] = useState(initialTransactions);
  const [error, setError] = useState('');
  const [toast, setToast] = useState('');
  const [connection, setConnection] = useState<'local' | 'connecting' | 'connected'>('local');
  const [connectedUser, setConnectedUser] = useState('');
  const [syncing, setSyncing] = useState(false);
  const [cacheOwner, setCacheOwner] = useState('');
  const [cacheReady, setCacheReady] = useState(false);
  const syncInFlight = useRef(false);
  const lastSyncAt = useRef(0);

  const budgetCategories = useMemo(() => categories.filter((item) => !item.system), [categories]);
  const categorySuggestion = useMemo(
    () => kind === 'expense' ? suggestCategory(description, categories, transactions, editingTransaction?.id) : null,
    [categories, description, editingTransaction?.id, kind, transactions],
  );
  const suggestedCategory = categorySuggestion
    ? categories.find((item) => item.id === categorySuggestion.categoryId) ?? null
    : null;
  useEffect(() => {
    if (kind === 'expense' && categorySuggestion && !categoryWasManuallySelected) {
      setCategoryFrom(categorySuggestion.categoryId);
    }
  }, [categorySuggestion, categoryWasManuallySelected, kind]);
  const usageScores = useMemo<UsageScores>(() => {
    const result: UsageScores = { categories: {}, accounts: {} };
    transactions.forEach((transaction, index) => {
      const weight = Math.pow(0.92, index);
      [transaction.categoryFrom, transaction.categoryTo].forEach((id) => {
        if (id && id !== SYSTEM_INCOME && id !== SYSTEM_EXPENSE) result.categories[id] = (result.categories[id] ?? 0) + weight;
      });
      [transaction.accountFrom, transaction.accountTo].forEach((id) => {
        if (id) result.accounts[id] = (result.accounts[id] ?? 0) + weight;
      });
    });
    return result;
  }, [transactions]);
  const selectedFromCategory = categories.find((item) => item.id === categoryFrom) ?? categories[0];
  const selectedToCategory = categories.find((item) => item.id === categoryTo) ?? categories[0];
  const selectedFromAccount = accounts.find((item) => item.id === accountFrom) ?? accounts[0];
  const selectedToAccount = accounts.find((item) => item.id === accountTo) ?? accounts[0];
  const pageTitle: Record<Screen, string> = { recent: 'Recent transactions', add: editingTransaction ? 'Edit transaction' : 'New transaction', budget: 'Budget availability', setup: 'Sheet setup' };

  function categoryId(name: string | null) {
    if (name === 'Income') return SYSTEM_INCOME;
    if (name === 'Expense') return SYSTEM_EXPENSE;
    return name ?? undefined;
  }

  function applySnapshot(snapshot: FinanceSnapshot) {
    const nextCategories: Category[] = snapshot.budgets.map((budget) => {
      const monthly = Number(budget.monthlyEur ?? 0);
      const spent = Number(budget.expenseEur ?? 0);
      const remaining = Number(budget.remainingEur ?? 0);
      return {
        id: budget.name,
        name: budget.name,
        monthly,
        rollover: remaining - monthly + spent,
        adjustment: 0,
        spent,
      };
    });
    nextCategories.push(
      { id: SYSTEM_INCOME, name: 'Income', monthly: 0, rollover: 0, adjustment: 0, spent: 0, system: 'source' },
      { id: SYSTEM_EXPENSE, name: 'Expense', monthly: 0, rollover: 0, adjustment: 0, spent: 0, system: 'sink' },
    );
    const nextAccounts: Account[] = snapshot.accounts.map((account) => {
      const type = String(account.type ?? '').toLowerCase();
      const kind: Account['kind'] = type.includes('card') ? 'card' : type.includes('cash') ? 'cash' : type.includes('saving') ? 'savings' : 'bank';
      return {
        id: account.name,
        name: account.name,
        currency: account.currency,
        kind,
        balanceNative: account.balanceNative,
        reconciliationNative: account.reconciliationNative,
      };
    });
    const nextTransactions: Transaction[] = snapshot.recentTransactions.map((transaction) => {
      let transactionKind: TransactionKind = 'expense';
      if (transaction.fromCategory === 'Income' && transaction.fromAccount === 'Income') transactionKind = 'income';
      else if (transaction.fromCategory === 'Balance Transfer' && transaction.toCategory === 'Balance Transfer') transactionKind = 'transfer';
      else if (transaction.fromAccount === 'Reallocation' && transaction.toAccount === 'Reallocation') transactionKind = 'reallocate';
      return {
        id: transaction.transactionId ?? `sheet-${transaction.row}`,
        kind: transactionKind,
        description: transaction.description,
        amount: Number(transaction.amount ?? 0),
        currency: transaction.currency ?? 'EUR',
        date: transaction.date ?? localDateIso(),
        dateLabel: transaction.date ? formatTransactionDate(transaction.date) : '—',
        toAmount: transaction.toAmount ?? undefined,
        sourceRow: transaction.row,
        revision: transaction.revision,
        categoryFrom: categoryId(transaction.fromCategory),
        categoryTo: categoryId(transaction.toCategory),
        accountFrom: transaction.fromAccount ?? undefined,
        accountTo: transaction.toAccount ?? undefined,
      };
    });
    setCategories(nextCategories);
    setAccounts(nextAccounts);
    setTransactions((current) => mergeTransactions(current, nextTransactions));
    const firstCategory = nextCategories.find((item) => !item.system);
    if (firstCategory && !nextCategories.some((item) => item.id === categoryFrom)) setCategoryFrom(firstCategory.id);
    if (firstCategory && !nextCategories.some((item) => item.id === categoryTo)) {
      setCategoryTo(nextCategories.find((item) => !item.system && item.id !== firstCategory.id)?.id ?? firstCategory.id);
    }
    if (nextAccounts[0] && !nextAccounts.some((item) => item.id === accountFrom)) {
      setAccountFrom(nextAccounts[0].id);
      setCurrency(nextAccounts[0].currency);
    }
    if (nextAccounts[0] && !nextAccounts.some((item) => item.id === accountTo)) {
      setAccountTo(nextAccounts[1]?.id ?? nextAccounts[0].id);
    }
  }

  async function hydrateCache(owner: string) {
    setCacheOwner(owner);
    setCacheReady(false);
    try {
      const cached = await loadFinanceCache(owner);
      if (!cached) return false;
      setCategories(cached.categories);
      setAccounts(cached.accounts);
      setTransactions(cached.transactions);
      lastSyncAt.current = Date.parse(cached.updatedAt) || 0;
      const firstCategory = cached.categories.find((item) => !item.system);
      if (firstCategory && !cached.categories.some((item) => item.id === categoryFrom)) setCategoryFrom(firstCategory.id);
      if (firstCategory && !cached.categories.some((item) => item.id === categoryTo)) setCategoryTo(firstCategory.id);
      if (cached.accounts[0] && !cached.accounts.some((item) => item.id === accountFrom)) {
        setAccountFrom(cached.accounts[0].id);
        setCurrency(cached.accounts[0].currency);
      }
      if (cached.accounts[0] && !cached.accounts.some((item) => item.id === accountTo)) setAccountTo(cached.accounts[1]?.id ?? cached.accounts[0].id);
      setCacheReady(true);
      return true;
    } catch {
      return false;
    }
  }

  async function syncSheet(recentCount = 80) {
    if (syncInFlight.current) return;
    syncInFlight.current = true;
    setSyncing(true);
    try {
      const snapshot = await fetchFinanceSnapshot(localDateIso(), recentCount);
      applySnapshot(snapshot);
      setCacheReady(true);
      lastSyncAt.current = Date.now();
    } catch (syncError) {
      setError(syncError instanceof Error ? syncError.message : String(syncError));
    } finally {
      syncInFlight.current = false;
      setSyncing(false);
    }
  }

  async function connectSheet() {
    if (!gatewayConfigured) {
      setError('Add the public Google client and Apps Script deployment IDs to the app build environment.');
      return;
    }
    setConnection('connecting');
    setError('');
    try {
      const email = await signInToGateway();
      const hasCache = await hydrateCache(email);
      setConnectedUser(email);
      setConnection('connected');
      setToast('Live sheet connected · loading in background');
      void syncSheet(hasCache ? 80 : 200);
      setTimeout(() => setToast(''), 2400);
    } catch (connectError) {
      setConnection('local');
      setError(connectError instanceof Error ? connectError.message : String(connectError));
    }
  }

  async function disconnectSheet() {
    const owner = connectedUser;
    await signOutFromGateway();
    if (owner) await clearFinanceCache(owner);
    setConnection('local');
    setConnectedUser('');
    setCategories(initialCategories);
    setAccounts(initialAccounts);
    setTransactions(initialTransactions);
    setCacheOwner('');
    setCacheReady(false);
    lastSyncAt.current = 0;
  }

  useEffect(() => {
    if (!gatewayConfigured) return;
    restoreGatewayUser().then(async (email) => {
      if (!email) return;
      setConnectedUser(email);
      const hasCache = await hydrateCache(email);
      setConnection('connected');
      void syncSheet(hasCache ? 80 : 200);
    }).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (connection !== 'connected' || !cacheOwner || !cacheReady) return;
    const timeout = setTimeout(() => {
      void saveFinanceCache(cacheOwner, { categories, accounts, transactions }).catch(() => undefined);
    }, 250);
    return () => clearTimeout(timeout);
  }, [accounts, cacheOwner, cacheReady, categories, connection, transactions]);

  function resetEntryForm() {
    const firstCategory = budgetCategories[0];
    const secondCategory = budgetCategories[1] ?? firstCategory;
    const firstAccount = accounts[0];
    const secondAccount = accounts[1] ?? firstAccount;
    setEditingTransaction(null);
    setKind('expense');
    setAmount('');
    setDescription('');
    setQuickEntry('');
    setDate(localDateIso());
    setToAmount('');
    setNote('');
    setShowMore(false);
    setCategoryWasManuallySelected(false);
    if (firstCategory) setCategoryFrom(firstCategory.id);
    if (secondCategory) setCategoryTo(secondCategory.id);
    if (firstAccount) { setAccountFrom(firstAccount.id); setCurrency(firstAccount.currency); }
    if (secondAccount) setAccountTo(secondAccount.id);
    setError('');
  }

  function changeKind(next: TransactionKind) {
    setKind(next);
    if (!editingTransaction) setDescription(typeDefaults[next]);
    setError('');
    setShowMore(false);
    setCategoryWasManuallySelected(false);
    const firstCategory = budgetCategories[0];
    const secondCategory = budgetCategories[1] ?? firstCategory;
    const firstAccount = accounts[0];
    const secondAccount = accounts[1] ?? firstAccount;
    if (next === 'expense') {
      if (firstCategory) setCategoryFrom(firstCategory.id);
      if (firstAccount) { setAccountFrom(firstAccount.id); setCurrency(firstAccount.currency); }
    } else if (next === 'income') {
      if (firstCategory) setCategoryTo(firstCategory.id);
      if (firstAccount) setAccountTo(firstAccount.id);
    } else if (next === 'transfer') {
      if (firstAccount) { setAccountFrom(firstAccount.id); setCurrency(firstAccount.currency); }
      if (secondAccount) setAccountTo(secondAccount.id);
    } else {
      if (firstCategory) setCategoryFrom(firstCategory.id);
      if (secondCategory) setCategoryTo(secondCategory.id);
    }
  }

  function changeScreen(next: Screen) {
    if (next === 'add' && screen !== 'add') resetEntryForm();
    setScreen(next);
    if (next === 'recent' && connection === 'connected' && Date.now() - lastSyncAt.current > 60_000) {
      void syncSheet();
    }
  }

  function beginEdit(transaction: Transaction) {
    if (connection === 'connected' && !transaction.sourceRow) {
      setToast('Refresh the live sheet before editing this entry.');
      setTimeout(() => setToast(''), 2400);
      return;
    }
    const parsed = parseQuickEntry(transaction.description);
    const parsedAccount = parsed ? resolveAccountQuery(parsed.accountQuery, accounts) : null;
    const firstCategory = budgetCategories[0];
    const firstAccount = accounts[0];
    const originalFromAccount = accounts.find((item) => item.id === transaction.accountFrom);
    const originalToAccount = accounts.find((item) => item.id === transaction.accountTo);
    const hasConversion = transaction.kind === 'transfer' && originalFromAccount?.currency !== originalToAccount?.currency && Boolean(transaction.toAmount);
    setEditingTransaction(transaction);
    setKind(parsed ? 'expense' : transaction.kind);
    setAmount(String(parsed?.amount ?? transaction.amount));
    setCurrency(parsed?.currency ?? transaction.currency);
    setDescription(parsed?.description ?? transaction.description);
    setQuickEntry(parsed ? transaction.description : '');
    setDate(transaction.date || localDateIso());
    setCategoryFrom(transaction.categoryFrom ?? firstCategory?.id ?? categoryFrom);
    setCategoryTo(transaction.categoryTo ?? firstCategory?.id ?? categoryTo);
    setAccountFrom(parsedAccount?.id ?? transaction.accountFrom ?? firstAccount?.id ?? accountFrom);
    setAccountTo(transaction.accountTo ?? firstAccount?.id ?? accountTo);
    setToAmount(hasConversion ? String(transaction.toAmount) : '');
    setCategoryWasManuallySelected(!parsed && Boolean(transaction.categoryFrom));
    setShowMore(hasConversion);
    setError(parsed && !parsedAccount ? `Account “${parsed.accountQuery}” is not available on this phone.` : '');
    setScreen('add');
  }

  function openPicker(target: PickerTarget, title: string) { setPicker({ target, title }); }

  function selectPickerValue(value: string) {
    if (!picker) return;
    if (picker.target === 'categoryFrom') {
      setCategoryFrom(value);
      if (kind === 'expense') setCategoryWasManuallySelected(true);
    }
    if (picker.target === 'categoryTo') setCategoryTo(value);
    if (picker.target === 'accountFrom') {
      setAccountFrom(value);
      const account = accounts.find((item) => item.id === value);
      if (account) setCurrency(account.currency);
    }
    if (picker.target === 'accountTo') setAccountTo(value);
    if (picker.target === 'currency') setCurrency(value as Currency);
    setPicker(null);
    setError('');
  }

  function applyQuickEntry() {
    const parsed = parseQuickEntry(quickEntry);
    if (!parsed) {
      setError('Use: PHP 2,287.43 / h seabank / cleaner for aug 30');
      return;
    }
    const account = resolveAccountQuery(parsed.accountQuery, accounts);
    if (!account) {
      setError(`Could not match account “${parsed.accountQuery}”. Check the account name in Setup.`);
      return;
    }
    setKind('expense');
    setAmount(parsed.amount.toFixed(2));
    setCurrency(parsed.currency);
    setAccountFrom(account.id);
    setDescription(parsed.description);
    setCategoryWasManuallySelected(false);
    if (!editingTransaction) setDate(localDateIso());
    const suggestion = suggestCategory(parsed.description, categories, transactions, editingTransaction?.id);
    if (suggestion) setCategoryFrom(suggestion.categoryId);
    setError('');
    setToast(`Filled amount, ${account.name}, description${suggestion ? ' and category' : ''}`);
    setTimeout(() => setToast(''), 2200);
  }

  function chooseOtherDate() {
    if (Platform.OS === 'android') {
      DateTimePickerAndroid.open({
        value: dateFromIso(date),
        mode: 'date',
        onChange: (_event, selected) => {
          if (selected) setDate(localDateIso(selected));
        },
      });
      return;
    }
    setShowIosDatePicker(true);
  }

  function applyBudgetEffect(items: Category[], transaction: Transaction, direction: 1 | -1) {
    return items.map((item) => {
      if (transaction.kind === 'expense' && item.id === transaction.categoryFrom) {
        return { ...item, spent: item.spent + transaction.amount * direction };
      }
      if (transaction.kind === 'income' && item.id === transaction.categoryTo) {
        return { ...item, adjustment: item.adjustment + transaction.amount * direction };
      }
      if (transaction.kind === 'reallocate' && item.id === transaction.categoryFrom) {
        return { ...item, adjustment: item.adjustment - transaction.amount * direction };
      }
      if (transaction.kind === 'reallocate' && item.id === transaction.categoryTo) {
        return { ...item, adjustment: item.adjustment + transaction.amount * direction };
      }
      return item;
    });
  }

  async function saveTransaction() {
    const parsed = Number(amount.includes('.') ? amount.replaceAll(',', '') : amount.replace(',', '.'));
    if (!Number.isFinite(parsed) || parsed <= 0) { setError('Enter an amount greater than zero.'); return; }
    if (!description.trim()) { setError('Add a description. Categories are selected separately.'); return; }
    if (kind === 'transfer' && accountFrom === accountTo) { setError('Choose two different accounts for a transfer.'); return; }
    if (kind === 'reallocate' && categoryFrom === categoryTo) { setError('Choose two different budget categories.'); return; }
    const enteredToAmount = toAmount.trim() ? Number(toAmount.includes('.') ? toAmount.replaceAll(',', '') : toAmount.replace(',', '.')) : null;
    const crossCurrencyTransfer = kind === 'transfer' && selectedFromAccount.currency !== selectedToAccount.currency;
    const parsedToAmount = crossCurrencyTransfer ? enteredToAmount : null;
    if (crossCurrencyTransfer && (!parsedToAmount || parsedToAmount <= 0)) {
      setError(`Enter the amount received in ${selectedToAccount.currency}.`);
      setShowMore(true);
      return;
    }

    const originalId = editingTransaction?.id;
    const existingUuid = originalId && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(originalId) ? originalId : null;
    const transactionId = existingUuid ?? uuidV7();
    const draft: Transaction = {
      id: transactionId,
      kind,
      description: description.trim(),
      amount: parsed,
      currency,
      date,
      dateLabel: formatTransactionDate(date),
      ...(parsedToAmount === null ? {} : { toAmount: parsedToAmount }),
      ...(editingTransaction?.sourceRow ? { sourceRow: editingTransaction.sourceRow } : {}),
      ...(editingTransaction?.revision ? { revision: editingTransaction.revision } : {}),
    };
    if (kind === 'expense') {
      draft.categoryFrom = categoryFrom; draft.categoryTo = SYSTEM_EXPENSE; draft.accountFrom = accountFrom;
    } else if (kind === 'income') {
      draft.categoryFrom = SYSTEM_INCOME; draft.categoryTo = categoryTo; draft.accountTo = accountTo;
    } else if (kind === 'transfer') {
      draft.accountFrom = accountFrom; draft.accountTo = accountTo;
    } else {
      draft.categoryFrom = categoryFrom; draft.categoryTo = categoryTo;
    }
    const payload = {
      transactionId,
      kind,
      date,
      description: draft.description,
      amount: parsed,
      ...(kind === 'reallocate' ? {} : { currency }),
      ...(parsedToAmount === null ? {} : { toAmount: parsedToAmount }),
      ...(kind === 'expense' || kind === 'reallocate' ? { fromCategory: selectedFromCategory.name } : {}),
      ...(kind === 'income' || kind === 'reallocate' ? { toCategory: selectedToCategory.name } : {}),
      ...(kind === 'expense' || kind === 'transfer' ? { fromAccount: selectedFromAccount.name } : {}),
      ...(kind === 'income' || kind === 'transfer' ? { toAccount: selectedToAccount.name } : {}),
    };

    if (connection === 'connected') {
      const pending = { ...draft, syncStatus: 'pending' as const };
      setTransactions((current) => editingTransaction
        ? current.map((item) => item.id === originalId ? pending : item)
        : [pending, ...current]);
      setEditingTransaction(null);
      setError('');
      setToAmount('');
      setScreen('recent');
      setToast(editingTransaction ? 'Updating the live sheet in the background…' : 'Saving to the live sheet in the background…');
      setTimeout(() => setToast(''), 2400);
      const request = editingTransaction
        ? updateGatewayTransaction({
          ...payload,
          row: editingTransaction.sourceRow ?? 0,
          ...(existingUuid ? { expectedTransactionId: existingUuid } : {}),
          ...(editingTransaction.revision ? { expectedRevision: editingTransaction.revision } : {}),
          expectedDate: editingTransaction.date,
          expectedDescription: editingTransaction.description,
          expectedAmount: editingTransaction.amount,
        })
        : submitGatewayTransaction(payload);
      void request.then(() => {
        setTransactions((current) => current.map((item) => item.id === transactionId ? { ...item, syncStatus: undefined } : item));
        setToast(editingTransaction ? 'Transaction updated in live sheet' : kind === 'reallocate' ? 'Budget reallocated in live sheet' : 'Transaction saved to live sheet');
        setTimeout(() => setToast(''), 2400);
        void syncSheet();
      }).catch((saveError) => {
        setTransactions((current) => current.map((item) => item.id === transactionId ? { ...item, syncStatus: 'failed' } : item));
        setError(saveError instanceof Error ? saveError.message : String(saveError));
        setToast('Could not sync transaction · check Setup');
        setTimeout(() => setToast(''), 3000);
      });
      return;
    }

    setCategories((current) => applyBudgetEffect(
      editingTransaction ? applyBudgetEffect(current, editingTransaction, -1) : current,
      draft,
      1,
    ));
    setTransactions((current) => editingTransaction
      ? current.map((item) => item.id === originalId ? draft : item)
      : [draft, ...current]);
    setEditingTransaction(null);
    setError('');
    setToast(editingTransaction ? 'Transaction updated locally' : kind === 'reallocate' ? 'Budget reallocated locally' : 'Transaction saved locally');
    setScreen('recent');
    setTimeout(() => setToast(''), 2400);
  }

  function renderEntryFields() {
    if (kind === 'expense') return (
      <>
        <ChoiceCard label="Budget category" value={selectedFromCategory.name} meta="Fixed list from Budget Definitions" icon="▤" onPress={() => openPicker('categoryFrom', 'Choose budget category')} />
        <BudgetStrip category={selectedFromCategory} />
        <ChoiceCard label="Paid from account" value={`${selectedFromAccount.name} · ${selectedFromAccount.currency}`} icon="▣" onPress={() => openPicker('accountFrom', 'Choose payment account')} />
        <FlowRow label="Category columns" flow={`${selectedFromCategory.name} → Expense`} />
      </>
    );
    if (kind === 'income') return (
      <>
        <ChoiceCard label="Fund budget category" value={selectedToCategory.name} meta={`${money(available(selectedToCategory))} currently available`} icon="▤" onPress={() => openPicker('categoryTo', 'Choose funded category')} />
        <ChoiceCard label="Deposit to account" value={`${selectedToAccount.name} · ${selectedToAccount.currency}`} icon="▣" onPress={() => openPicker('accountTo', 'Choose deposit account')} />
        <FlowRow label="Category columns" flow={`Income → ${selectedToCategory.name}`} />
      </>
    );
    if (kind === 'transfer') return (
      <>
        <ChoiceCard label="From account" value={`${selectedFromAccount.name} · ${selectedFromAccount.currency}`} icon="↑" onPress={() => openPicker('accountFrom', 'Choose source account')} />
        <ChoiceCard label="To account" value={`${selectedToAccount.name} · ${selectedToAccount.currency}`} icon="↓" onPress={() => openPicker('accountTo', 'Choose destination account')} />
        <FlowRow label="Budget effect" flow="No category movement" />
      </>
    );
    return (
      <>
        <ChoiceCard label="From budget category" value={selectedFromCategory.name} meta={`${money(available(selectedFromCategory))} available · ${money(selectedFromCategory.monthly)}/mo + rollover`} icon="↑" onPress={() => openPicker('categoryFrom', 'Move budget from')} />
        <ChoiceCard label="To budget category" value={selectedToCategory.name} meta={`${money(available(selectedToCategory))} available · ${money(selectedToCategory.monthly)}/mo + rollover`} icon="↓" onPress={() => openPicker('categoryTo', 'Move budget to')} />
        <FlowRow label="Account effect" flow="None · budget only" />
      </>
    );
  }

  function renderAddScreen() {
    const today = localDateIso();
    const yesterday = relativeDateIso(1);
    const otherDate = date !== today && date !== yesterday;
    return (
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={styles.flex}>
        <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.screenContent}>
          <FieldLabel>Quick entry</FieldLabel>
          <View style={styles.quickEntryRow}>
            <TextInput value={quickEntry} onChangeText={(value) => { setQuickEntry(value); setError(''); }} onSubmitEditing={applyQuickEntry} placeholder="PHP 2,287.43 / h seabank / cleaner for aug 30" placeholderTextColor={colors.muted} returnKeyType="done" style={[styles.input, styles.quickEntryInput]} accessibilityLabel="Quick entry" />
            <Pressable onPress={applyQuickEntry} style={({ pressed }) => [styles.applyButton, pressed && styles.pressed]}><Text style={styles.applyButtonText}>Fill</Text></Pressable>
          </View>
          <Text style={styles.quickHint}>Also accepts “th gotyme” as “h gotyme”.</Text>
          <TypeTabs value={kind} onChange={changeKind} />
          <View style={styles.amountRow}>
            <View style={styles.amountField}>
              <FieldLabel>Amount</FieldLabel>
              <TextInput value={amount} onChangeText={(value) => { setAmount(value); setError(''); }} keyboardType="decimal-pad" selectTextOnFocus style={[styles.input, styles.amountInput]} accessibilityLabel="Amount" />
            </View>
            <View style={styles.currencyField}>
              <FieldLabel>Currency</FieldLabel>
              <Pressable onPress={() => openPicker('currency', 'Choose currency')} style={styles.currencyButton}><Text style={styles.currencyText}>{currency}</Text><Text style={styles.chevron}>⌄</Text></Pressable>
            </View>
          </View>
          <FieldLabel>Description</FieldLabel>
          <TextInput value={description} onChangeText={(value) => { setDescription(value); setError(''); }} placeholder="Merchant, payer or reason" placeholderTextColor={colors.muted} style={styles.input} accessibilityLabel="Description" />
          {kind === 'expense' && suggestedCategory ? <Pressable onPress={() => { setCategoryFrom(suggestedCategory.id); setCategoryWasManuallySelected(false); }} style={styles.suggestionBanner}><Text style={styles.suggestionBannerText}>Suggested category: {suggestedCategory.name}</Text><Text style={styles.suggestionReason}>{categorySuggestion?.reason}</Text></Pressable> : null}
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.suggestions}>
            {descriptionSuggestions.map((item) => <Pressable key={item} onPress={() => { setDescription(item); setCategoryWasManuallySelected(false); }} style={({ pressed }) => [styles.chip, pressed && styles.pressed]}><Text style={styles.chipText}>{item}</Text></Pressable>)}
          </ScrollView>
          {renderEntryFields()}
          <FieldLabel>Date</FieldLabel>
          <View style={styles.dateRow}>
            <Pressable onPress={() => setDate(today)} style={[styles.dateButton, date === today && styles.dateButtonActive]}><Text style={[styles.dateText, date === today && styles.dateTextActive]}>Today · {formatActualDate(today)}</Text></Pressable>
            <Pressable onPress={() => setDate(yesterday)} style={[styles.dateButton, date === yesterday && styles.dateButtonActive]}><Text style={[styles.dateText, date === yesterday && styles.dateTextActive]}>Yesterday · {formatActualDate(yesterday)}</Text></Pressable>
            <Pressable onPress={chooseOtherDate} style={[styles.dateButton, otherDate && styles.dateButtonActive]}><Text style={[styles.dateText, otherDate && styles.dateTextActive]}>{otherDate ? formatActualDate(date) : 'Other date'}</Text></Pressable>
          </View>
          <Text style={styles.timezoneHint}>{deviceTimeZone()}</Text>
          <Pressable onPress={() => setShowMore((value) => !value)} style={styles.moreButton}><Text style={styles.moreText}>{showMore ? 'Hide details' : 'More details'}</Text></Pressable>
          {showMore ? <View style={styles.morePanel}>
            <FieldLabel>Sheet note / reference</FieldLabel><TextInput value={note} onChangeText={setNote} placeholder="Optional" placeholderTextColor={colors.muted} style={styles.input} />
            <FieldLabel>To amount (currency conversion only)</FieldLabel><TextInput value={toAmount} onChangeText={(value) => { setToAmount(value); setError(''); }} placeholder={kind === 'transfer' && selectedFromAccount.currency !== selectedToAccount.currency ? `Amount received in ${selectedToAccount.currency}` : 'Same as amount'} placeholderTextColor={colors.muted} keyboardType="decimal-pad" style={styles.input} />
          </View> : null}
          {error ? <Text style={styles.errorText}>{error}</Text> : null}
          <Pressable onPress={saveTransaction} style={({ pressed }) => [styles.primaryButton, pressed && styles.primaryButtonPressed]}><Text style={styles.primaryButtonText}>{editingTransaction ? 'Save changes' : kind === 'reallocate' ? 'Reallocate budget' : 'Save transaction'}</Text></Pressable>
          {editingTransaction ? <Pressable onPress={() => { resetEntryForm(); setScreen('recent'); }} style={styles.cancelEditButton}><Text style={styles.moreText}>Cancel editing</Text></Pressable> : null}
          <Text style={styles.localHint}>{connection === 'connected' ? 'Writes queue instantly and sync to the live sheet in the background' : 'Local demo · changes reset when the app closes'}</Text>
        </ScrollView>
      </KeyboardAvoidingView>
    );
  }

  function renderRecentScreen() {
    return <ScrollView contentContainerStyle={styles.screenContent}>
      <View style={styles.syncLine}><View style={[styles.syncDot, connection === 'connected' && styles.connectedDot]} /><Text style={styles.syncText}>{connection === 'connected' ? `${syncing ? 'Updating from live sheet in background' : 'Live sheet up to date'}${connectedUser ? ` · ${connectedUser}` : ''}` : 'Local demo data · no sheet connected'}</Text></View>
      <View style={styles.listGap}>{transactions.map((item) => <TransactionRow key={item.id} transaction={item} categories={categories} accounts={accounts} onPress={() => beginEdit(item)} />)}</View>
    </ScrollView>;
  }

  function renderBudgetScreen() {
    const totalAvailable = budgetCategories.reduce((sum, item) => sum + available(item), 0);
    return <ScrollView contentContainerStyle={styles.screenContent}>
      <View style={styles.totalCard}><Text style={styles.totalLabel}>Total available across budgets</Text><Text style={styles.totalValue}>{money(totalAvailable)}</Text><Text style={styles.totalMeta}>August · includes accumulated rollover and reallocations</Text></View>
      <Text style={styles.sectionLabel}>SPENDING CATEGORIES</Text>
      <View style={styles.listGap}>{budgetCategories.map((item) => <BudgetRow key={item.id} category={item} />)}</View>
      <Text style={styles.sectionLabel}>SYSTEM CATEGORIES</Text>
      <View style={styles.systemCard}><View style={styles.systemRow}><Text style={styles.systemTitle}>Income</Text><Text style={styles.systemMeta}>Source · funds budget categories</Text></View><View style={styles.systemDivider} /><View style={styles.systemRow}><Text style={styles.systemTitle}>Expense</Text><Text style={styles.systemMeta}>Sink · receives spending</Text></View></View>
    </ScrollView>;
  }

  function renderSetupScreen() {
    const connected = connection === 'connected';
    const connecting = connection === 'connecting';
    return <ScrollView contentContainerStyle={styles.screenContent}>
      <View style={[styles.notConnectedCard, connected && styles.connectedCard]}><View style={[styles.notConnectedIcon, connected && styles.connectedIcon]}><Text style={[styles.notConnectedIconText, connected && styles.connectedIconText]}>{connected ? '✓' : '⌁'}</Text></View><View style={styles.setupCopy}><Text style={styles.notConnectedTitle}>{connected ? 'Live sheet connected' : connecting ? 'Connecting to Google…' : 'Google Sheets not connected'}</Text><Text style={styles.notConnectedText}>{connected ? `Signed in as ${connectedUser}. Categories, accounts and recent transactions come from the live workbook.` : 'Sign in with an allowed Google account to read and write the configured live workbook.'}</Text></View></View>
      {error ? <Text style={styles.errorText}>{error}</Text> : null}
      <Pressable disabled={connecting} onPress={connected ? disconnectSheet : connectSheet} style={({ pressed }) => [connected ? styles.secondaryButton : styles.primaryButton, connecting && styles.disabledButton, pressed && styles.pressed]}><Text style={connected ? styles.secondaryButtonText : styles.primaryButtonText}>{connected ? 'Disconnect' : connecting ? 'Connecting…' : 'Connect Google Sheets'}</Text></Pressable>
      {!gatewayConfigured ? <Text style={styles.localHint}>This build is missing its public OAuth client and Apps Script deployment IDs.</Text> : null}
      <Text style={styles.sectionLabel}>SHEET MAPPING</Text>
      <View style={styles.setupCard}><SetupRow icon="▦" title="Transactions" detail="Date, description, from/to category, account and currency" value={connected ? 'Live' : 'Ready'} /><View style={styles.systemDivider} /><SetupRow icon="▤" title="Budget Definitions" detail="Fixed categories, monthly amounts and accumulated availability" value={connected ? 'Live' : 'Ready'} /><View style={styles.systemDivider} /><SetupRow icon="€" title="Transaction currency" detail="Defaults to the selected source account; can be overridden" value={currency} /></View>
      <Text style={styles.sectionLabel}>CURRENT BEHAVIOR</Text>
      <View style={styles.setupCard}><SetupRow icon="✦" title="Smart suggestions" detail="Learns from prior descriptions and category keywords" value="On" /><View style={styles.systemDivider} /><SetupRow icon="◷" title="Timezone" detail="Uses the phone timezone; Madrid is the fallback" value={deviceTimeZone()} /><View style={styles.systemDivider} /><SetupRow icon="⌁" title="Background sync" detail="Reads and writes do not block transaction entry" value="On" /><View style={styles.systemDivider} /><SetupRow icon="◉" title="Write safety" detail="Google identity, allowlist, UUIDv7 idempotency and script lock" value="On" /></View>
      <Text style={styles.versionText}>Presto Presupuesto · live integration 1.3.0</Text>
    </ScrollView>;
  }

  const selectedPickerValue = picker?.target === 'categoryFrom' ? categoryFrom : picker?.target === 'categoryTo' ? categoryTo : picker?.target === 'accountFrom' ? accountFrom : picker?.target === 'accountTo' ? accountTo : currency;

  return (
    <SafeAreaProvider initialMetrics={initialWindowMetrics}>
      <SafeAreaView edges={['top', 'right', 'bottom', 'left']} style={styles.safeArea}>
      <StatusBar style="dark" />
      <Header title={pageTitle[screen]} connected={connection === 'connected'} />
      <View style={styles.flex}>{screen === 'add' ? renderAddScreen() : null}{screen === 'recent' ? renderRecentScreen() : null}{screen === 'budget' ? renderBudgetScreen() : null}{screen === 'setup' ? renderSetupScreen() : null}</View>
      {toast ? <View style={styles.toast} accessibilityLiveRegion="polite"><Text style={styles.toastIcon}>✓</Text><Text style={styles.toastText}>{toast}</Text></View> : null}
      <BottomNav value={screen} onChange={changeScreen} />
      {Platform.OS === 'ios' && showIosDatePicker ? <Modal visible transparent animationType="fade" onRequestClose={() => setShowIosDatePicker(false)}>
        <View style={styles.dateModalRoot}><View style={styles.dateModalCard}>
          <Text style={styles.sheetTitle}>Choose date</Text>
          <DateTimePicker value={dateFromIso(date)} mode="date" display="inline" onChange={(_event, selected) => { if (selected) setDate(localDateIso(selected)); }} />
          <Pressable onPress={() => setShowIosDatePicker(false)} style={styles.primaryButton}><Text style={styles.primaryButtonText}>Done</Text></Pressable>
        </View></View>
      </Modal> : null}
      <PickerModal picker={picker} categories={categories} accounts={accounts} usageScores={usageScores} selected={selectedPickerValue} onSelect={selectPickerValue} onClose={() => setPicker(null)} />
      </SafeAreaView>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({
  safeArea: { flex: 1, backgroundColor: colors.background },
  flex: { flex: 1 },
  header: { paddingHorizontal: 20, paddingTop: 14, paddingBottom: 14, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' },
  eyebrow: { color: colors.primary, fontSize: 10, fontWeight: '700', letterSpacing: 1.2, marginBottom: 3 },
  headerTitle: { color: colors.text, fontSize: 24, fontWeight: '700' },
  localBadge: { flexDirection: 'row', alignItems: 'center', gap: 6, backgroundColor: colors.surfaceSoft, borderRadius: 99, paddingHorizontal: 10, paddingVertical: 7 },
  localDot: { width: 7, height: 7, borderRadius: 4, backgroundColor: colors.warning },
  connectedDot: { backgroundColor: colors.positive },
  localBadgeText: { color: colors.muted, fontSize: 11, fontWeight: '600' },
  screenContent: { paddingHorizontal: 16, paddingBottom: 28 },
  typeTabs: { backgroundColor: colors.surfaceSoft, borderRadius: radius.medium, padding: 4, flexDirection: 'row', marginBottom: 18 },
  typeTab: { flex: 1, minHeight: 42, alignItems: 'center', justifyContent: 'center', borderRadius: radius.small, paddingHorizontal: 3 },
  typeTabActive: { backgroundColor: colors.surface, shadowColor: '#18202A', shadowOpacity: 0.12, shadowRadius: 4, shadowOffset: { width: 0, height: 1 }, elevation: 2 },
  typeTabText: { color: colors.muted, fontSize: 12, fontWeight: '600' },
  typeTabTextActive: { color: colors.text },
  pressed: { opacity: 0.7 },
  fieldLabel: { color: colors.muted, fontSize: 12, fontWeight: '600', marginLeft: 2, marginBottom: 7 },
  quickEntryRow: { flexDirection: 'row', gap: 8, marginBottom: 5 }, quickEntryInput: { flex: 1, marginBottom: 0, fontSize: 13 }, applyButton: { minWidth: 62, borderRadius: radius.medium, backgroundColor: colors.primarySoft, alignItems: 'center', justifyContent: 'center' }, applyButtonText: { color: colors.primary, fontSize: 13, fontWeight: '800' }, quickHint: { color: colors.muted, fontSize: 10, marginLeft: 2, marginBottom: 16 },
  amountRow: { flexDirection: 'row', gap: 10 }, amountField: { flex: 1 }, currencyField: { width: 98 },
  input: { minHeight: 50, borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, color: colors.text, paddingHorizontal: 13, fontSize: 16, marginBottom: 13 },
  amountInput: { fontSize: 28, fontWeight: '700' },
  currencyButton: { minHeight: 50, borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, paddingHorizontal: 13, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' },
  currencyText: { color: colors.text, fontSize: 16, fontWeight: '600' }, chevron: { color: colors.muted, fontSize: 18 },
  suggestions: { gap: 8, paddingBottom: 14 },
  chip: { borderWidth: 1, borderColor: colors.line, backgroundColor: colors.surface, borderRadius: 99, paddingHorizontal: 12, paddingVertical: 8 }, chipText: { color: colors.text, fontSize: 12, fontWeight: '500' },
  choiceCard: { minHeight: 72, flexDirection: 'row', alignItems: 'center', gap: 11, borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, paddingHorizontal: 12, paddingVertical: 11, marginBottom: 10 },
  choiceIcon: { width: 36, height: 36, borderRadius: 11, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.primarySoft }, choiceIconText: { color: colors.primary, fontSize: 17, fontWeight: '700' },
  choiceCopy: { flex: 1 }, choiceLabel: { color: colors.muted, fontSize: 11, fontWeight: '600', marginBottom: 2 }, choiceValue: { color: colors.text, fontSize: 15, fontWeight: '700' }, choiceMeta: { color: colors.muted, fontSize: 11, marginTop: 3 }, changeText: { color: colors.primary, fontSize: 12, fontWeight: '600' },
  budgetStrip: { marginTop: -4, marginBottom: 12, backgroundColor: colors.surfaceSoft, borderBottomLeftRadius: radius.medium, borderBottomRightRadius: radius.medium, paddingHorizontal: 12, paddingVertical: 11 },
  rowBetween: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', gap: 10 }, availableText: { color: colors.positive, fontSize: 12, fontWeight: '700' }, smallMuted: { color: colors.muted, fontSize: 11 },
  track: { height: 6, borderRadius: 4, backgroundColor: colors.line, overflow: 'hidden', marginTop: 9 }, fill: { height: '100%', borderRadius: 4, backgroundColor: colors.primary }, fillDanger: { backgroundColor: colors.danger },
  flowRow: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 10, paddingHorizontal: 2, marginBottom: 14 }, flowValue: { color: colors.text, fontSize: 12, fontWeight: '700', flexShrink: 1, textAlign: 'right' },
  dateRow: { flexDirection: 'row', flexWrap: 'wrap', gap: 8, marginBottom: 4 },
  timezoneHint: { color: colors.muted, fontSize: 10, marginTop: 5, marginLeft: 2 }, suggestionBanner: { borderWidth: 1, borderColor: colors.primary, backgroundColor: colors.primarySoft, borderRadius: radius.medium, paddingHorizontal: 12, paddingVertical: 9, marginTop: -5, marginBottom: 12 }, suggestionBannerText: { color: colors.primary, fontSize: 12, fontWeight: '800' }, suggestionReason: { color: colors.muted, fontSize: 10, marginTop: 2 }, cancelEditButton: { minHeight: 44, alignItems: 'center', justifyContent: 'center' }, dateButton: { borderWidth: 1, borderColor: colors.line, borderRadius: 99, backgroundColor: colors.surface, paddingHorizontal: 16, paddingVertical: 10 }, dateButtonActive: { borderColor: colors.primary, backgroundColor: colors.primary }, dateText: { color: colors.text, fontSize: 13, fontWeight: '600' }, dateTextActive: { color: '#FFFFFF' },
  moreButton: { minHeight: 46, alignItems: 'center', justifyContent: 'center', marginVertical: 4 }, moreText: { color: colors.primary, fontSize: 14, fontWeight: '600' }, morePanel: { marginTop: 4 }, errorText: { color: colors.danger, fontSize: 13, fontWeight: '600', marginBottom: 10 },
  primaryButton: { minHeight: 54, borderRadius: radius.medium, backgroundColor: colors.primary, alignItems: 'center', justifyContent: 'center' }, primaryButtonPressed: { backgroundColor: '#274EA4' }, primaryButtonText: { color: '#FFFFFF', fontSize: 16, fontWeight: '700' }, secondaryButton: { minHeight: 54, borderWidth: 1, borderColor: colors.primary, borderRadius: radius.medium, backgroundColor: colors.surface, alignItems: 'center', justifyContent: 'center' }, secondaryButtonText: { color: colors.primary, fontSize: 16, fontWeight: '700' }, disabledButton: { opacity: 0.55 }, localHint: { textAlign: 'center', color: colors.muted, fontSize: 11, marginTop: 10 },
  syncLine: { flexDirection: 'row', alignItems: 'center', gap: 7, marginBottom: 12 }, syncDot: { width: 8, height: 8, borderRadius: 4, backgroundColor: colors.warning }, syncText: { color: colors.muted, fontSize: 12 }, listGap: { gap: 9 },
  syncFailed: { color: colors.danger, fontWeight: '700' },
  transactionRow: { minHeight: 86, flexDirection: 'row', alignItems: 'center', gap: 11, borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, padding: 12 }, transactionIcon: { width: 42, height: 42, borderRadius: 13, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.surfaceSoft }, transactionIconText: { color: colors.primary, fontSize: 18, fontWeight: '700' }, transactionCopy: { flex: 1 }, transactionTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, transactionMeta: { color: colors.muted, fontSize: 11, marginTop: 3 }, transactionDate: { color: colors.muted, fontSize: 10, marginTop: 3 }, transactionAmount: { color: colors.text, fontSize: 14, fontWeight: '700' }, transactionSide: { alignItems: 'flex-end', gap: 6 }, editText: { color: colors.primary, fontSize: 11, fontWeight: '700' }, positive: { color: colors.positive }, negative: { color: colors.danger },
  totalCard: { backgroundColor: colors.primary, borderRadius: radius.large, padding: 18, marginBottom: 20 }, totalLabel: { color: '#DCE7FF', fontSize: 12, fontWeight: '600' }, totalValue: { color: '#FFFFFF', fontSize: 32, fontWeight: '800', marginVertical: 5 }, totalMeta: { color: '#DCE7FF', fontSize: 11 },
  sectionLabel: { color: colors.muted, fontSize: 10, fontWeight: '800', letterSpacing: 1.1, marginTop: 18, marginBottom: 9, marginLeft: 2 }, budgetCard: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, padding: 13 }, budgetName: { color: colors.text, fontSize: 14, fontWeight: '700' }, budgetAmount: { color: colors.text, fontSize: 15, fontWeight: '800' }, budgetMeta: { color: colors.muted, fontSize: 11, marginTop: 4 },
  systemCard: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, paddingHorizontal: 13 }, systemRow: { paddingVertical: 13, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 12 }, systemTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, systemMeta: { color: colors.muted, fontSize: 11, flex: 1, textAlign: 'right' }, systemDivider: { height: StyleSheet.hairlineWidth, backgroundColor: colors.line },
  notConnectedCard: { flexDirection: 'row', gap: 12, backgroundColor: '#FFF8E7', borderWidth: 1, borderColor: '#EDD9A6', borderRadius: radius.large, padding: 15, marginBottom: 6 }, notConnectedIcon: { width: 40, height: 40, borderRadius: 13, alignItems: 'center', justifyContent: 'center', backgroundColor: '#F7E7B9' }, notConnectedIconText: { color: colors.warning, fontSize: 20, fontWeight: '700' }, notConnectedTitle: { color: colors.text, fontSize: 14, fontWeight: '800', marginBottom: 3 }, notConnectedText: { color: colors.muted, fontSize: 12, lineHeight: 17 },
  connectedCard: { backgroundColor: '#EFFAF4', borderColor: '#B9E2CA' }, connectedIcon: { backgroundColor: '#D8F1E2' }, connectedIconText: { color: colors.positive },
  setupCard: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, paddingHorizontal: 13 }, setupRow: { flexDirection: 'row', alignItems: 'center', gap: 11, paddingVertical: 13 }, setupIcon: { width: 34, height: 34, borderRadius: 10, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.surfaceSoft }, setupCopy: { flex: 1 }, setupTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, setupDetail: { color: colors.muted, fontSize: 11, marginTop: 3, lineHeight: 15 }, setupValue: { color: colors.primary, fontSize: 11, fontWeight: '700' }, versionText: { color: colors.muted, fontSize: 11, textAlign: 'center', marginTop: 24 },
  bottomNav: { flexDirection: 'row', gap: 5, borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.line, backgroundColor: colors.surface, paddingHorizontal: 10, paddingTop: 7, paddingBottom: 6 }, navItem: { flex: 1, minHeight: 52, borderRadius: radius.medium, alignItems: 'center', justifyContent: 'center', gap: 2 }, navItemActive: { backgroundColor: colors.primarySoft }, navIcon: { color: colors.muted, fontSize: 17, fontWeight: '700' }, navText: { color: colors.muted, fontSize: 10, fontWeight: '600' }, navTextActive: { color: colors.primary },
  toast: { position: 'absolute', left: 16, right: 16, bottom: 82, minHeight: 50, borderRadius: radius.medium, backgroundColor: colors.text, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 9, paddingHorizontal: 14, zIndex: 20 }, toastIcon: { color: '#FFFFFF', fontSize: 16, fontWeight: '800' }, toastText: { color: '#FFFFFF', fontSize: 13, fontWeight: '700' },
  searchWrap: { paddingHorizontal: 14, paddingBottom: 10 }, searchInput: { minHeight: 46, borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.background, color: colors.text, paddingHorizontal: 13, fontSize: 15 }, noResults: { color: colors.muted, textAlign: 'center', paddingVertical: 28 },
  dateModalRoot: { flex: 1, justifyContent: 'center', padding: 22, backgroundColor: colors.overlay }, dateModalCard: { borderRadius: radius.large, backgroundColor: colors.surface, padding: 18 },
  modalRoot: { flex: 1, justifyContent: 'flex-end' }, modalBackdrop: { position: 'absolute', top: 0, right: 0, bottom: 0, left: 0, backgroundColor: colors.overlay }, sheet: { maxHeight: '78%', backgroundColor: colors.surface, borderTopLeftRadius: 26, borderTopRightRadius: 26, paddingTop: 9 }, sheetHandle: { width: 42, height: 5, borderRadius: 3, backgroundColor: colors.line, alignSelf: 'center', marginBottom: 8 }, sheetHeader: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', paddingHorizontal: 18, paddingBottom: 12 }, sheetTitle: { color: colors.text, fontSize: 20, fontWeight: '800' }, closeButton: { paddingHorizontal: 10, paddingVertical: 8 }, closeText: { color: colors.primary, fontSize: 13, fontWeight: '700' }, sheetList: { flexGrow: 0 }, sheetListContent: { paddingHorizontal: 14, paddingBottom: 24, gap: 7 }, optionRow: { minHeight: 62, flexDirection: 'row', alignItems: 'center', gap: 12, borderRadius: radius.medium, paddingHorizontal: 12, backgroundColor: colors.background }, optionRowActive: { backgroundColor: colors.primarySoft }, radio: { width: 20, height: 20, borderRadius: 10, borderWidth: 2, borderColor: colors.line, alignItems: 'center', justifyContent: 'center' }, radioActive: { borderColor: colors.primary }, radioDot: { width: 10, height: 10, borderRadius: 5, backgroundColor: colors.primary }, optionCopy: { flex: 1 }, optionTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, optionMeta: { color: colors.muted, fontSize: 11, marginTop: 3 },
});
