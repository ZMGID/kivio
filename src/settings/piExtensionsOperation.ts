import type { PiExtensionInventory } from '../api/externalCliSettings'
import { externalCliSettingsApi } from '../api/externalCliSettings'
import { createWindowStore, useWindowStore } from '../utils/windowStore'

export type PiExtensionsOperationState = {
  inventory: PiExtensionInventory | null
  loading: boolean
  error: string | null
  query: string
  source: string
  busy: string | null
  result: string | null
}

const initialPiExtensionsState: PiExtensionsOperationState = {
  inventory: null,
  loading: true,
  error: null,
  query: '',
  source: '',
  busy: null,
  result: null,
}

const piExtensionsStore = createWindowStore(initialPiExtensionsState)
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
  clearInventoryOnError: boolean
  quiet: boolean
}

async function reloadPiExtensions(options: ReloadOptions) {
  const ticket = epoch
  const sequence = ++inventorySequence
  if (!options.quiet) {
    piExtensionsStore.setState((state) => ({
      ...state,
      loading: true,
      error: options.clearError ? null : state.error,
    }))
  }
  try {
    const inventory = await externalCliSettingsApi.piExtensionsInventory()
    if (ticket !== epoch || sequence !== inventorySequence) return
    piExtensionsStore.setState((state) => ({ ...state, inventory, loading: false }))
  } catch (error) {
    if (ticket !== epoch || sequence !== inventorySequence) return
    piExtensionsStore.setState((state) => ({
      ...state,
      loading: false,
      error: options.preserveExistingError && state.error ? state.error : errorMessage(error),
      inventory: options.clearInventoryOnError ? null : state.inventory,
    }))
  }
}

export function usePiExtensionsOperation() {
  return useWindowStore(piExtensionsStore)
}

export function resetPiExtensionsOperationState() {
  epoch += 1
  inventorySequence += 1
  piExtensionsStore.setState(initialPiExtensionsState)
}

/** Page return refreshes inventory without wiping an action error, result, or draft input. */
export function refreshPiExtensionsOnVisit() {
  const snapshot = piExtensionsStore.getSnapshot()
  if (snapshot.busy) return Promise.resolve()
  return reloadPiExtensions({
    clearError: false,
    preserveExistingError: true,
    clearInventoryOnError: snapshot.inventory == null,
    quiet: snapshot.inventory != null,
  })
}

export function refreshPiExtensionsNow() {
  return reloadPiExtensions({
    clearError: true,
    preserveExistingError: false,
    clearInventoryOnError: true,
    quiet: false,
  })
}

export function runPiExtensionAction(
  key: string,
  action: () => Promise<{ output?: string } | void>,
  doneLabel: string,
  options?: { clearSource?: boolean },
) {
  const ticket = epoch
  const submittedSource = piExtensionsStore.getSnapshot().source
  return piExtensionsStore.run(`${ticket}:${key}`, async () => {
    if (ticket !== epoch) return
    if (piExtensionsStore.getSnapshot().busy) return
    piExtensionsStore.setState((state) => ({ ...state, busy: key, result: null, error: null }))
    try {
      const next = await action()
      if (ticket !== epoch) return
      const output = next && 'output' in next && typeof next.output === 'string' ? next.output.trim() : ''
      piExtensionsStore.setState((state) => ({
        ...state,
        result: output || doneLabel,
        source: options?.clearSource && state.source === submittedSource ? '' : state.source,
      }))
      await reloadPiExtensions({
        clearError: false,
        preserveExistingError: false,
        clearInventoryOnError: true,
        quiet: false,
      })
    } catch (error) {
      if (ticket !== epoch) return
      piExtensionsStore.setState((state) => ({ ...state, error: errorMessage(error) }))
    } finally {
      if (ticket === epoch) {
        piExtensionsStore.setState((state) => ({
          ...state,
          busy: state.busy === key ? null : state.busy,
        }))
      }
    }
  })
}
