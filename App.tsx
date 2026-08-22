import { StatusBar } from 'expo-status-bar';
import { useMemo, useState } from 'react';
import {
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  SafeAreaView,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import {
  accounts,
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
import { colors, radius } from './src/theme';

type Screen = 'recent' | 'add' | 'budget' | 'setup';
type PickerTarget = 'categoryFrom' | 'categoryTo' | 'accountFrom' | 'accountTo' | 'currency';
type PickerState = { target: PickerTarget; title: string } | null;

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

function Header({ title }: { title: string }) {
  return (
    <View style={styles.header}>
      <View>
        <Text style={styles.eyebrow}>PRESTO PRESUPUESTO</Text>
        <Text style={styles.headerTitle}>{title}</Text>
      </View>
      <View style={styles.localBadge}>
        <View style={styles.localDot} />
        <Text style={styles.localBadgeText}>Local demo</Text>
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

function TransactionRow({ transaction, categories }: { transaction: Transaction; categories: Category[] }) {
  const isPositive = transaction.kind === 'income';
  const sign = transaction.kind === 'expense' ? '−' : transaction.kind === 'income' ? '+' : '';
  const icon = transaction.kind === 'expense' ? '↘' : transaction.kind === 'income' ? '↗' : transaction.kind === 'transfer' ? '⇄' : '⇆';
  let flow = '';
  if (transaction.kind === 'expense' || transaction.kind === 'income') {
    flow = `${categoryName(categories, transaction.categoryFrom)} → ${categoryName(categories, transaction.categoryTo)}`;
    const account = transaction.accountFrom ?? transaction.accountTo;
    if (account) flow += ` · ${accountName(account)}`;
  } else if (transaction.kind === 'transfer') {
    flow = `${accountName(transaction.accountFrom)} → ${accountName(transaction.accountTo)} · No budget change`;
  } else {
    flow = `${categoryName(categories, transaction.categoryFrom)} → ${categoryName(categories, transaction.categoryTo)} · Budget only`;
  }
  return (
    <View style={styles.transactionRow}>
      <View style={styles.transactionIcon}><Text style={styles.transactionIconText}>{icon}</Text></View>
      <View style={styles.transactionCopy}>
        <Text style={styles.transactionTitle}>{transaction.description}</Text>
        <Text style={styles.transactionMeta} numberOfLines={2}>{flow}</Text>
        <Text style={styles.transactionDate}>{transaction.dateLabel}</Text>
      </View>
      <Text style={[styles.transactionAmount, isPositive && styles.positive]}>{sign}{money(transaction.amount, transaction.currency)}</Text>
    </View>
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

function PickerModal({ picker, categories, selected, onSelect, onClose }: {
  picker: PickerState;
  categories: Category[];
  selected: string;
  onSelect: (value: string) => void;
  onClose: () => void;
}) {
  if (!picker) return null;
  const isCategory = picker.target === 'categoryFrom' || picker.target === 'categoryTo';
  const isAccount = picker.target === 'accountFrom' || picker.target === 'accountTo';
  const options = isCategory
    ? categories.filter((item) => !item.system).map((item) => ({ id: item.id, label: item.name, meta: `${money(available(item))} available · ${money(item.monthly)}/mo` }))
    : isAccount
      ? accounts.map((item) => ({ id: item.id, label: item.name, meta: item.currency }))
      : (['EUR', 'USD', 'GBP'] as Currency[]).map((item) => ({ id: item, label: item, meta: 'Currency' }));
  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <View style={styles.modalRoot}>
        <Pressable style={styles.modalBackdrop} onPress={onClose} accessibilityLabel="Close picker" />
        <View style={styles.sheet}>
          <View style={styles.sheetHandle} />
          <View style={styles.sheetHeader}>
            <Text style={styles.sheetTitle}>{picker.title}</Text>
            <Pressable onPress={onClose} style={styles.closeButton}><Text style={styles.closeText}>Close</Text></Pressable>
          </View>
          <ScrollView style={styles.sheetList} contentContainerStyle={styles.sheetListContent}>
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
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

export default function App() {
  const [screen, setScreen] = useState<Screen>('add');
  const [kind, setKind] = useState<TransactionKind>('expense');
  const [amount, setAmount] = useState('42.80');
  const [currency, setCurrency] = useState<Currency>('EUR');
  const [description, setDescription] = useState(typeDefaults.expense);
  const [categoryFrom, setCategoryFrom] = useState('food');
  const [categoryTo, setCategoryTo] = useState('home');
  const [accountFrom, setAccountFrom] = useState('visa');
  const [accountTo, setAccountTo] = useState('checking');
  const [dateLabel, setDateLabel] = useState('Today');
  const [note, setNote] = useState('');
  const [showMore, setShowMore] = useState(false);
  const [picker, setPicker] = useState<PickerState>(null);
  const [categories, setCategories] = useState(initialCategories);
  const [transactions, setTransactions] = useState(initialTransactions);
  const [error, setError] = useState('');
  const [toast, setToast] = useState('');

  const budgetCategories = useMemo(() => categories.filter((item) => !item.system), [categories]);
  const selectedFromCategory = categories.find((item) => item.id === categoryFrom) ?? categories[0];
  const selectedToCategory = categories.find((item) => item.id === categoryTo) ?? categories[0];
  const selectedFromAccount = accounts.find((item) => item.id === accountFrom) ?? accounts[0];
  const selectedToAccount = accounts.find((item) => item.id === accountTo) ?? accounts[0];
  const pageTitle: Record<Screen, string> = { recent: 'Recent transactions', add: 'New transaction', budget: 'Budget availability', setup: 'Sheet setup' };

  function changeKind(next: TransactionKind) {
    setKind(next);
    setDescription(typeDefaults[next]);
    setError('');
    setShowMore(false);
    if (next === 'expense') { setCategoryFrom('food'); setAccountFrom('visa'); }
    else if (next === 'income') { setCategoryTo('buffer'); setAccountTo('checking'); }
    else if (next === 'transfer') { setAccountFrom('checking'); setAccountTo('savings'); }
    else { setCategoryFrom('travel'); setCategoryTo('home'); }
  }

  function openPicker(target: PickerTarget, title: string) { setPicker({ target, title }); }

  function selectPickerValue(value: string) {
    if (!picker) return;
    if (picker.target === 'categoryFrom') setCategoryFrom(value);
    if (picker.target === 'categoryTo') setCategoryTo(value);
    if (picker.target === 'accountFrom') setAccountFrom(value);
    if (picker.target === 'accountTo') setAccountTo(value);
    if (picker.target === 'currency') setCurrency(value as Currency);
    setPicker(null);
    setError('');
  }

  function saveTransaction() {
    const parsed = Number(amount.replace(',', '.'));
    if (!Number.isFinite(parsed) || parsed <= 0) { setError('Enter an amount greater than zero.'); return; }
    if (!description.trim()) { setError('Add a description. Categories are selected separately.'); return; }
    if (kind === 'transfer' && accountFrom === accountTo) { setError('Choose two different accounts for a transfer.'); return; }
    if (kind === 'reallocate' && categoryFrom === categoryTo) { setError('Choose two different budget categories.'); return; }
    if (kind === 'reallocate' && available(selectedFromCategory) < parsed) { setError(`${selectedFromCategory.name} only has ${money(available(selectedFromCategory))} available.`); return; }

    const base: Transaction = { id: `local-${Date.now()}`, kind, description: description.trim(), amount: parsed, currency, dateLabel: dateLabel === 'Today' ? 'Today · just now' : 'Yesterday' };
    if (kind === 'expense') {
      base.categoryFrom = categoryFrom; base.categoryTo = SYSTEM_EXPENSE; base.accountFrom = accountFrom;
      setCategories((current) => current.map((item) => item.id === categoryFrom ? { ...item, spent: item.spent + parsed } : item));
    } else if (kind === 'income') {
      base.categoryFrom = SYSTEM_INCOME; base.categoryTo = categoryTo; base.accountTo = accountTo;
      setCategories((current) => current.map((item) => item.id === categoryTo ? { ...item, adjustment: item.adjustment + parsed } : item));
    } else if (kind === 'transfer') {
      base.accountFrom = accountFrom; base.accountTo = accountTo;
    } else {
      base.categoryFrom = categoryFrom; base.categoryTo = categoryTo;
      setCategories((current) => current.map((item) => item.id === categoryFrom ? { ...item, adjustment: item.adjustment - parsed } : item.id === categoryTo ? { ...item, adjustment: item.adjustment + parsed } : item));
    }
    setTransactions((current) => [base, ...current]);
    setError('');
    setToast(kind === 'reallocate' ? 'Budget reallocated locally' : 'Transaction saved locally');
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
    return (
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={styles.flex}>
        <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.screenContent}>
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
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.suggestions}>
            {descriptionSuggestions.map((item) => <Pressable key={item} onPress={() => setDescription(item)} style={({ pressed }) => [styles.chip, pressed && styles.pressed]}><Text style={styles.chipText}>{item}</Text></Pressable>)}
          </ScrollView>
          {renderEntryFields()}
          <FieldLabel>Date</FieldLabel>
          <View style={styles.dateRow}>{(['Today', 'Yesterday'] as const).map((item) => <Pressable key={item} onPress={() => setDateLabel(item)} style={[styles.dateButton, dateLabel === item && styles.dateButtonActive]}><Text style={[styles.dateText, dateLabel === item && styles.dateTextActive]}>{item}</Text></Pressable>)}</View>
          <Pressable onPress={() => setShowMore((value) => !value)} style={styles.moreButton}><Text style={styles.moreText}>{showMore ? 'Hide details' : 'More details'}</Text></Pressable>
          {showMore ? <View style={styles.morePanel}>
            <FieldLabel>Sheet note / reference</FieldLabel><TextInput value={note} onChangeText={setNote} placeholder="Optional" placeholderTextColor={colors.muted} style={styles.input} />
            <FieldLabel>To amount (currency conversion only)</FieldLabel><TextInput placeholder="Same as amount" placeholderTextColor={colors.muted} keyboardType="decimal-pad" style={styles.input} />
          </View> : null}
          {error ? <Text style={styles.errorText}>{error}</Text> : null}
          <Pressable onPress={saveTransaction} style={({ pressed }) => [styles.primaryButton, pressed && styles.primaryButtonPressed]}><Text style={styles.primaryButtonText}>{kind === 'reallocate' ? 'Reallocate budget' : 'Save transaction'}</Text></Pressable>
          <Text style={styles.localHint}>UX prototype · changes reset when the app closes</Text>
        </ScrollView>
      </KeyboardAvoidingView>
    );
  }

  function renderRecentScreen() {
    return <ScrollView contentContainerStyle={styles.screenContent}>
      <View style={styles.syncLine}><View style={styles.syncDot} /><Text style={styles.syncText}>Local demo data · no sheet connected</Text></View>
      <View style={styles.listGap}>{transactions.map((item) => <TransactionRow key={item.id} transaction={item} categories={categories} />)}</View>
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
    return <ScrollView contentContainerStyle={styles.screenContent}>
      <View style={styles.notConnectedCard}><View style={styles.notConnectedIcon}><Text style={styles.notConnectedIconText}>⌁</Text></View><View style={styles.setupCopy}><Text style={styles.notConnectedTitle}>Google Sheets not connected</Text><Text style={styles.notConnectedText}>This APK is intentionally UX-only. All entries stay in memory on this device.</Text></View></View>
      <Text style={styles.sectionLabel}>PLANNED SHEET MAPPING</Text>
      <View style={styles.setupCard}><SetupRow icon="▦" title="Transactions" detail="Date, description, from/to category, account and currency" value="Planned" /><View style={styles.systemDivider} /><SetupRow icon="▤" title="Budget Definitions" detail="Fixed categories, monthly amounts and rollover" value="Planned" /><View style={styles.systemDivider} /><SetupRow icon="€" title="Default currency" detail="Used for new transactions" value={currency} /></View>
      <Text style={styles.sectionLabel}>PROTOTYPE BEHAVIOR</Text>
      <View style={styles.setupCard}><SetupRow icon="✦" title="Smart suggestions" detail="Description suggestions are separate from category selection" value="On" /><View style={styles.systemDivider} /><SetupRow icon="⌁" title="Offline storage" detail="Not included in this UX build" value="Off" /><View style={styles.systemDivider} /><SetupRow icon="◉" title="App lock" detail="Biometric protection is not included yet" value="Off" /></View>
      <Text style={styles.versionText}>Presto Presupuesto · UX prototype 1.0.0</Text>
    </ScrollView>;
  }

  const selectedPickerValue = picker?.target === 'categoryFrom' ? categoryFrom : picker?.target === 'categoryTo' ? categoryTo : picker?.target === 'accountFrom' ? accountFrom : picker?.target === 'accountTo' ? accountTo : currency;

  return (
    <SafeAreaView style={styles.safeArea}>
      <StatusBar style="dark" />
      <Header title={pageTitle[screen]} />
      <View style={styles.flex}>{screen === 'add' ? renderAddScreen() : null}{screen === 'recent' ? renderRecentScreen() : null}{screen === 'budget' ? renderBudgetScreen() : null}{screen === 'setup' ? renderSetupScreen() : null}</View>
      {toast ? <View style={styles.toast} accessibilityLiveRegion="polite"><Text style={styles.toastIcon}>✓</Text><Text style={styles.toastText}>{toast}</Text></View> : null}
      <BottomNav value={screen} onChange={setScreen} />
      <PickerModal picker={picker} categories={categories} selected={selectedPickerValue} onSelect={selectPickerValue} onClose={() => setPicker(null)} />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safeArea: { flex: 1, backgroundColor: colors.background, paddingTop: Platform.OS === 'android' ? 24 : 0 },
  flex: { flex: 1 },
  header: { paddingHorizontal: 20, paddingTop: 14, paddingBottom: 14, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' },
  eyebrow: { color: colors.primary, fontSize: 10, fontWeight: '700', letterSpacing: 1.2, marginBottom: 3 },
  headerTitle: { color: colors.text, fontSize: 24, fontWeight: '700' },
  localBadge: { flexDirection: 'row', alignItems: 'center', gap: 6, backgroundColor: colors.surfaceSoft, borderRadius: 99, paddingHorizontal: 10, paddingVertical: 7 },
  localDot: { width: 7, height: 7, borderRadius: 4, backgroundColor: colors.warning },
  localBadgeText: { color: colors.muted, fontSize: 11, fontWeight: '600' },
  screenContent: { paddingHorizontal: 16, paddingBottom: 28 },
  typeTabs: { backgroundColor: colors.surfaceSoft, borderRadius: radius.medium, padding: 4, flexDirection: 'row', marginBottom: 18 },
  typeTab: { flex: 1, minHeight: 42, alignItems: 'center', justifyContent: 'center', borderRadius: radius.small, paddingHorizontal: 3 },
  typeTabActive: { backgroundColor: colors.surface, shadowColor: '#18202A', shadowOpacity: 0.12, shadowRadius: 4, shadowOffset: { width: 0, height: 1 }, elevation: 2 },
  typeTabText: { color: colors.muted, fontSize: 12, fontWeight: '600' },
  typeTabTextActive: { color: colors.text },
  pressed: { opacity: 0.7 },
  fieldLabel: { color: colors.muted, fontSize: 12, fontWeight: '600', marginLeft: 2, marginBottom: 7 },
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
  dateRow: { flexDirection: 'row', gap: 8, marginBottom: 4 }, dateButton: { borderWidth: 1, borderColor: colors.line, borderRadius: 99, backgroundColor: colors.surface, paddingHorizontal: 16, paddingVertical: 10 }, dateButtonActive: { borderColor: colors.primary, backgroundColor: colors.primary }, dateText: { color: colors.text, fontSize: 13, fontWeight: '600' }, dateTextActive: { color: '#FFFFFF' },
  moreButton: { minHeight: 46, alignItems: 'center', justifyContent: 'center', marginVertical: 4 }, moreText: { color: colors.primary, fontSize: 14, fontWeight: '600' }, morePanel: { marginTop: 4 }, errorText: { color: colors.danger, fontSize: 13, fontWeight: '600', marginBottom: 10 },
  primaryButton: { minHeight: 54, borderRadius: radius.medium, backgroundColor: colors.primary, alignItems: 'center', justifyContent: 'center' }, primaryButtonPressed: { backgroundColor: '#274EA4' }, primaryButtonText: { color: '#FFFFFF', fontSize: 16, fontWeight: '700' }, localHint: { textAlign: 'center', color: colors.muted, fontSize: 11, marginTop: 10 },
  syncLine: { flexDirection: 'row', alignItems: 'center', gap: 7, marginBottom: 12 }, syncDot: { width: 8, height: 8, borderRadius: 4, backgroundColor: colors.warning }, syncText: { color: colors.muted, fontSize: 12 }, listGap: { gap: 9 },
  transactionRow: { minHeight: 86, flexDirection: 'row', alignItems: 'center', gap: 11, borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, padding: 12 }, transactionIcon: { width: 42, height: 42, borderRadius: 13, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.surfaceSoft }, transactionIconText: { color: colors.primary, fontSize: 18, fontWeight: '700' }, transactionCopy: { flex: 1 }, transactionTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, transactionMeta: { color: colors.muted, fontSize: 11, marginTop: 3 }, transactionDate: { color: colors.muted, fontSize: 10, marginTop: 3 }, transactionAmount: { color: colors.text, fontSize: 14, fontWeight: '700' }, positive: { color: colors.positive }, negative: { color: colors.danger },
  totalCard: { backgroundColor: colors.primary, borderRadius: radius.large, padding: 18, marginBottom: 20 }, totalLabel: { color: '#DCE7FF', fontSize: 12, fontWeight: '600' }, totalValue: { color: '#FFFFFF', fontSize: 32, fontWeight: '800', marginVertical: 5 }, totalMeta: { color: '#DCE7FF', fontSize: 11 },
  sectionLabel: { color: colors.muted, fontSize: 10, fontWeight: '800', letterSpacing: 1.1, marginTop: 18, marginBottom: 9, marginLeft: 2 }, budgetCard: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, padding: 13 }, budgetName: { color: colors.text, fontSize: 14, fontWeight: '700' }, budgetAmount: { color: colors.text, fontSize: 15, fontWeight: '800' }, budgetMeta: { color: colors.muted, fontSize: 11, marginTop: 4 },
  systemCard: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, paddingHorizontal: 13 }, systemRow: { paddingVertical: 13, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 12 }, systemTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, systemMeta: { color: colors.muted, fontSize: 11, flex: 1, textAlign: 'right' }, systemDivider: { height: StyleSheet.hairlineWidth, backgroundColor: colors.line },
  notConnectedCard: { flexDirection: 'row', gap: 12, backgroundColor: '#FFF8E7', borderWidth: 1, borderColor: '#EDD9A6', borderRadius: radius.large, padding: 15, marginBottom: 6 }, notConnectedIcon: { width: 40, height: 40, borderRadius: 13, alignItems: 'center', justifyContent: 'center', backgroundColor: '#F7E7B9' }, notConnectedIconText: { color: colors.warning, fontSize: 20, fontWeight: '700' }, notConnectedTitle: { color: colors.text, fontSize: 14, fontWeight: '800', marginBottom: 3 }, notConnectedText: { color: colors.muted, fontSize: 12, lineHeight: 17 },
  setupCard: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.medium, backgroundColor: colors.surface, paddingHorizontal: 13 }, setupRow: { flexDirection: 'row', alignItems: 'center', gap: 11, paddingVertical: 13 }, setupIcon: { width: 34, height: 34, borderRadius: 10, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.surfaceSoft }, setupCopy: { flex: 1 }, setupTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, setupDetail: { color: colors.muted, fontSize: 11, marginTop: 3, lineHeight: 15 }, setupValue: { color: colors.primary, fontSize: 11, fontWeight: '700' }, versionText: { color: colors.muted, fontSize: 11, textAlign: 'center', marginTop: 24 },
  bottomNav: { flexDirection: 'row', gap: 5, borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.line, backgroundColor: colors.surface, paddingHorizontal: 10, paddingTop: 7, paddingBottom: Platform.OS === 'android' ? 10 : 6 }, navItem: { flex: 1, minHeight: 52, borderRadius: radius.medium, alignItems: 'center', justifyContent: 'center', gap: 2 }, navItemActive: { backgroundColor: colors.primarySoft }, navIcon: { color: colors.muted, fontSize: 17, fontWeight: '700' }, navText: { color: colors.muted, fontSize: 10, fontWeight: '600' }, navTextActive: { color: colors.primary },
  toast: { position: 'absolute', left: 16, right: 16, bottom: 82, minHeight: 50, borderRadius: radius.medium, backgroundColor: colors.text, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 9, paddingHorizontal: 14, zIndex: 20 }, toastIcon: { color: '#FFFFFF', fontSize: 16, fontWeight: '800' }, toastText: { color: '#FFFFFF', fontSize: 13, fontWeight: '700' },
  modalRoot: { flex: 1, justifyContent: 'flex-end' }, modalBackdrop: { position: 'absolute', top: 0, right: 0, bottom: 0, left: 0, backgroundColor: colors.overlay }, sheet: { maxHeight: '78%', backgroundColor: colors.surface, borderTopLeftRadius: 26, borderTopRightRadius: 26, paddingTop: 9 }, sheetHandle: { width: 42, height: 5, borderRadius: 3, backgroundColor: colors.line, alignSelf: 'center', marginBottom: 8 }, sheetHeader: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', paddingHorizontal: 18, paddingBottom: 12 }, sheetTitle: { color: colors.text, fontSize: 20, fontWeight: '800' }, closeButton: { paddingHorizontal: 10, paddingVertical: 8 }, closeText: { color: colors.primary, fontSize: 13, fontWeight: '700' }, sheetList: { flexGrow: 0 }, sheetListContent: { paddingHorizontal: 14, paddingBottom: 24, gap: 7 }, optionRow: { minHeight: 62, flexDirection: 'row', alignItems: 'center', gap: 12, borderRadius: radius.medium, paddingHorizontal: 12, backgroundColor: colors.background }, optionRowActive: { backgroundColor: colors.primarySoft }, radio: { width: 20, height: 20, borderRadius: 10, borderWidth: 2, borderColor: colors.line, alignItems: 'center', justifyContent: 'center' }, radioActive: { borderColor: colors.primary }, radioDot: { width: 10, height: 10, borderRadius: 5, backgroundColor: colors.primary }, optionCopy: { flex: 1 }, optionTitle: { color: colors.text, fontSize: 14, fontWeight: '700' }, optionMeta: { color: colors.muted, fontSize: 11, marginTop: 3 },
});
