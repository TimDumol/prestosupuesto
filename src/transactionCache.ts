import type { Transaction } from './domain';

export function mergeTransactions(current: Transaction[], remote: Transaction[]) {
  const remoteIds = new Set(remote.map((item) => item.id));
  const remoteRows = new Set(remote.map((item) => item.sourceRow).filter((row): row is number => row !== undefined));
  const retained = current.filter((item) => (
    !remoteIds.has(item.id)
    && (item.sourceRow === undefined || !remoteRows.has(item.sourceRow))
  ));
  return [
    ...retained.filter((item) => item.syncStatus),
    ...remote,
    ...retained.filter((item) => !item.syncStatus),
  ].slice(0, 200);
}
