import type { PiSkillInventory } from '../api/externalCliSettings'
import { externalCliSettingsApi } from '../api/externalCliSettings'
import { createWindowStore, useWindowStore } from '../utils/windowStore'

export type PiSkillsOperationState = {
  inventory: PiSkillInventory | null
  loading: boolean
  error: string | null
  query: string
  busy: string | null
  result: string | null
}

const initialPiSkillsState: PiSkillsOperationState = {
  inventory: null,
  loading: true,
  error: null,
  query: '',
  busy: null,
  result: null,
}

const piSkillsStore = createWindowStore(initialPiSkillsState)
let epoch = 0
let inventorySequence = 0

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message
  if (typeof error === 'string' && error.trim()) return error
  return String(error)
}

type ReloadOptions = {
  clearError: boolean
  preserveExistingError: boolean
  quiet: boolean
}

async function reloadPiSkills(options: ReloadOptions) {
  const ticket = epoch
  const sequence = ++inventorySequence
  if (!options.quiet) {
    piSkillsStore.setState((state) => ({
      ...state,
      loading: true,
      error: options.clearError ? null : state.error,
    }))
  }
  try {
    const inventory = await externalCliSettingsApi.piSkillsInventory()
    if (ticket !== epoch || sequence !== inventorySequence) return
    piSkillsStore.setState((state) => ({ ...state, inventory, loading: false }))
  } catch (error) {
    if (ticket !== epoch || sequence !== inventorySequence) return
    piSkillsStore.setState((state) => ({
      ...state,
      loading: false,
      error: options.preserveExistingError && state.error ? state.error : errorMessage(error),
    }))
  }
}

export function usePiSkillsOperation() {
  return useWindowStore(piSkillsStore)
}

export function resetPiSkillsOperationState() {
  epoch += 1
  inventorySequence += 1
  piSkillsStore.setState(initialPiSkillsState)
}

export function refreshPiSkillsOnVisit() {
  const snapshot = piSkillsStore.getSnapshot()
  if (snapshot.busy) return Promise.resolve()
  return reloadPiSkills({
    clearError: false,
    preserveExistingError: true,
    quiet: snapshot.inventory != null,
  })
}

export function refreshPiSkillsNow() {
  return reloadPiSkills({
    clearError: true,
    preserveExistingError: false,
    quiet: false,
  })
}

export function runPiSkillAction(key: string, action: () => Promise<void>, success?: string) {
  const ticket = epoch
  return piSkillsStore.run(`${ticket}:${key}`, async () => {
    if (ticket !== epoch) return
    if (piSkillsStore.getSnapshot().busy) return
    piSkillsStore.setState((state) => ({ ...state, busy: key, error: null, result: null }))
    try {
      await action()
      if (ticket !== epoch) return
      if (success) piSkillsStore.setState((state) => ({ ...state, result: success }))
      await reloadPiSkills({ clearError: false, preserveExistingError: false, quiet: false })
    } catch (error) {
      if (ticket !== epoch) return
      piSkillsStore.setState((state) => ({ ...state, error: errorMessage(error) }))
    } finally {
      if (ticket === epoch) {
        piSkillsStore.setState((state) => ({
          ...state,
          busy: state.busy === key ? null : state.busy,
        }))
      }
    }
  })
}
