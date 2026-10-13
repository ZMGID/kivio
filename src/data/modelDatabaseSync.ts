import { api } from '../api/tauri'
import { applyModelDatabaseSnapshot } from './modelMatching'

/** Subscribe before reading, so updates cannot be lost while a window starts. */
export async function startModelDatabaseSync() {
  const stop = await api.onModelDatabaseChanged(applyModelDatabaseSnapshot)
  try {
    applyModelDatabaseSnapshot(await api.getModelDatabase())
    return stop
  } catch (error) {
    stop()
    throw error
  }
}

export async function updateModelDatabase() {
  const snapshot = await api.updateModelDatabase()
  applyModelDatabaseSnapshot(snapshot)
  return snapshot
}
