import {
  GoogleOneTapSignIn,
  isCancelledResponse,
  isNoSavedCredentialFoundResponse,
  isSuccessResponse,
} from 'react-native-nitro-google-signin';
import type { Currency, TransactionKind } from './domain';

const SCOPES = [
  'https://www.googleapis.com/auth/spreadsheets',
  'https://www.googleapis.com/auth/userinfo.email',
];

export const gatewayConfigured = Boolean(
  process.env.EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID
  && process.env.EXPO_PUBLIC_APPS_SCRIPT_DEPLOYMENT_ID,
);

let configured = false;

function configure() {
  if (configured) return;
  const webClientId = process.env.EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID;
  if (!webClientId) throw new Error('Missing EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID.');
  GoogleOneTapSignIn.configure({ webClientId, scopes: SCOPES, offlineAccess: false });
  configured = true;
}

export async function signInToGateway(): Promise<string> {
  configure();
  await GoogleOneTapSignIn.checkPlayServices();
  let response = await GoogleOneTapSignIn.signIn();
  if (isNoSavedCredentialFoundResponse(response)) response = await GoogleOneTapSignIn.createAccount();
  if (isNoSavedCredentialFoundResponse(response)) response = await GoogleOneTapSignIn.presentExplicitSignIn();
  if (isCancelledResponse(response)) throw new Error('Google sign-in was cancelled.');
  if (!isSuccessResponse(response)) throw new Error('Google sign-in did not complete.');
  const email = response.data.user.email;
  if (!email) throw new Error('The selected Google account did not provide an email address.');
  return email;
}

export async function restoreGatewayUser() {
  configure();
  const current = GoogleOneTapSignIn.getCurrentUser();
  return current?.user.email ?? null;
}

export async function signOutFromGateway() {
  configure();
  await GoogleOneTapSignIn.signOut();
}

async function accessToken() {
  configure();
  return (await GoogleOneTapSignIn.getTokens()).accessToken;
}

async function runGateway<T>(functionName: string, parameter?: object): Promise<T> {
  const deploymentId = process.env.EXPO_PUBLIC_APPS_SCRIPT_DEPLOYMENT_ID;
  if (!deploymentId) throw new Error('Missing EXPO_PUBLIC_APPS_SCRIPT_DEPLOYMENT_ID.');
  async function request(token: string) {
    // The host is fixed to Google's API; only the deployment ID is build-time configuration.
    return fetch(`https://script.googleapis.com/v1/scripts/${deploymentId}:run`, { // foxguard: ignore[js/no-ssrf]
      method: 'POST',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({
        function: functionName,
        ...(parameter ? { parameters: [parameter] } : {}),
      }),
    });
  }
  let token = await accessToken();
  let response = await request(token);
  if (response.status === 401) {
    await GoogleOneTapSignIn.clearCachedAccessToken(token);
    token = await accessToken();
    response = await request(token);
  }
  const operation = await response.json();
  if (!response.ok) throw new Error(operation?.error?.message ?? `Google API error ${response.status}.`);
  if (operation.error) {
    throw new Error(operation.error.details?.[0]?.errorMessage ?? operation.error.message ?? 'Gateway failed.');
  }
  return operation.response?.result as T;
}

export type GatewayBudget = {
  name: string;
  monthlyEur: number | null;
  expenseEur: number | null;
  remainingEur: number | null;
};

export type GatewayAccount = {
  name: string;
  type: string | null;
  currency: Currency;
  balanceNative: number | null;
  actualBalanceNative: number | null;
  reconciliationNative: number | null;
  balanceEur: number | null;
  actualBalanceEur: number | null;
};

export type GatewayTransaction = {
  row: number;
  date: string | null;
  description: string;
  amount: number | null;
  currency: Currency | null;
  toAmount: number | null;
  fromCategory: string | null;
  toCategory: string | null;
  fromAccount: string | null;
  toAccount: string | null;
  transactionId: string | null;
  createdAt: string | null;
  createdBy: string | null;
  revision: string;
};

export type FinanceSnapshot = {
  generatedAt: string;
  asOf: string;
  budgetMonth: string;
  transactionCount: number;
  recentTransactions: GatewayTransaction[];
  budgets: GatewayBudget[];
  accounts: GatewayAccount[];
};

export type SubmitTransaction = {
  transactionId: string;
  kind: TransactionKind;
  date: string;
  description: string;
  amount: number;
  currency?: Currency;
  toAmount?: number;
  fromCategory?: string;
  toCategory?: string;
  fromAccount?: string;
  toAccount?: string;
};

export type UpdateTransaction = SubmitTransaction & {
  row: number;
  expectedTransactionId?: string;
  expectedRevision?: string;
  expectedDate: string;
  expectedDescription: string;
  expectedAmount: number;
};

export function fetchFinanceSnapshot(asOf: string, recentCount = 50) {
  return runGateway<FinanceSnapshot>('getFinanceSnapshot', { asOf, recentCount });
}

export function submitGatewayTransaction(payload: SubmitTransaction) {
  return runGateway<{ duplicate: boolean; transaction: GatewayTransaction }>('submitTransaction', payload);
}

export function updateGatewayTransaction(payload: UpdateTransaction) {
  return runGateway<{ updated: boolean; transaction: GatewayTransaction }>('updateTransaction', payload);
}
