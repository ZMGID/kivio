import { useSyncExternalStore } from 'react'
import { getModelDatabaseSnapshot, subscribeModelDatabase } from './modelMatching'

/** Subscribe only views whose rendered model metadata depends on the catalog. */
export function useModelDatabase() {
  return useSyncExternalStore(subscribeModelDatabase, getModelDatabaseSnapshot, getModelDatabaseSnapshot)
}
