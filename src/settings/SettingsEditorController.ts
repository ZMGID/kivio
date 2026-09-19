import type { Settings, SettingsSnapshot, SettingsVersion } from '../api/tauri'
import { isSettingsVersionConflict } from '../api/tauri'
import {
  acceptSettingsSave,
  createSettingsEditorState,
  receiveSettingsSnapshot,
  updateSettingsEditorDraft,
  type SettingsEditorState,
  type SettingsMergeConflict,
} from './rebaseSettingsDraft'
import { stableStringify } from './utils'
export interface SettingsCloseOptions {
  /** Ordinary close waits for flush; navigation can leave while saving continues. */
  waitForSave?: boolean
}

export interface SettingsEditorPort {
  peek(): SettingsSnapshot | null
  load(): Promise<SettingsSnapshot>
  refresh(): Promise<SettingsSnapshot>
  save(draft: Settings, version: SettingsVersion): Promise<SettingsSnapshot>
  subscribe(listener: (snapshot: SettingsSnapshot) => void): () => void
  import?(path: string, version: SettingsVersion): Promise<SettingsSnapshot>
}

export interface SettingsEditorView {
  settings: Settings | null
  loading: boolean
  loadError: string
  saveError: string
  hasUnsavedChanges: boolean
  conflicts: readonly SettingsMergeConflict[]
}

/** Owns the editable settings lifecycle; callers only edit, flush, and observe a snapshot. */
export class SettingsEditorController {
  private editor: SettingsEditorState | null = null
  private version: SettingsVersion | null = null
  private view: SettingsEditorView = {
    settings: null,
    loading: true,
    loadError: '',
    saveError: '',
    hasUnsavedChanges: false,
    conflicts: [],
  }
  private listeners = new Set<() => void>()
  private unsubscribe: (() => void) | null = null
  private autosaveTimer: ReturnType<typeof setTimeout> | null = null
  private saveFlight: Promise<boolean> | null = null
  private closeFlight: Promise<void> | null = null
  private pendingSnapshot: SettingsSnapshot | null = null
  private loadGeneration = 0
  private disposed = false

  constructor(
    private readonly port: SettingsEditorPort,
    private readonly onSettingsChange: () => void = () => {},
  ) {}

  get snapshot(): SettingsEditorView { return this.view }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  private publish(patch: Partial<SettingsEditorView> = {}) {
    const editor = this.editor
    this.view = {
      ...this.view,
      settings: editor?.draft ?? null,
      hasUnsavedChanges: !!editor && stableStringify(editor.draft) !== stableStringify(editor.acknowledgedDraft),
      conflicts: editor?.conflicts ?? [],
      ...patch,
    }
    for (const listener of this.listeners) listener()
  }

  private accept(snapshot: SettingsSnapshot) {
    if (this.saveFlight) {
      this.pendingSnapshot = snapshot
      return
    }
    if (this.version?.epoch === snapshot.version.epoch && this.version.revision >= snapshot.version.revision) return
    this.version = snapshot.version
    this.editor = this.editor
      ? receiveSettingsSnapshot(this.editor, snapshot.settings)
      : createSettingsEditorState(snapshot.settings)
    this.publish({ loading: false, loadError: '', saveError: this.conflictError() })
    if (this.view.hasUnsavedChanges && this.editor.conflicts.length === 0) this.scheduleSave()
  }

  private conflictError(): string {
    return this.editor?.conflicts.length
      ? `Settings conflict: ${this.editor.conflicts.map((conflict) => conflict.path).join(', ')}`
      : ''
  }

  /** Starts cache-first loading and subscribes to future canonical snapshots. */
  start() {
    this.disposed = false
    this.closeFlight = null
    const generation = ++this.loadGeneration
    this.unsubscribe?.()
    this.unsubscribe = this.port.subscribe((snapshot) => this.accept(snapshot))
    const cached = this.port.peek()
    if (cached) this.accept(cached)
    else this.publish({ loading: true, loadError: '' })
    void (cached ? this.port.refresh() : this.port.load())
      .then((snapshot) => {
        if (this.disposed || generation !== this.loadGeneration) return
        this.accept(snapshot)
      })
      .catch((error) => {
        if (this.disposed || generation !== this.loadGeneration || cached) return
        const message = error instanceof Error ? error.message : String(error)
        this.publish({ loading: false, loadError: message || 'Unknown error' })
      })
  }

  edit(update: Settings | ((current: Settings) => Settings)) {
    if (!this.editor) return
    const next = typeof update === 'function' ? update(this.editor.draft) : update
    if (next === this.editor.draft) return
    this.editor = updateSettingsEditorDraft(this.editor, next)
    this.publish({ saveError: this.conflictError() })
    if (this.editor.conflicts.length === 0 && this.view.hasUnsavedChanges) this.scheduleSave()
  }

  private scheduleSave() {
    this.clearAutosave()
    this.autosaveTimer = setTimeout(() => {
      this.autosaveTimer = null
      void this.flush()
    }, 400)
  }

  private clearAutosave() {
    if (this.autosaveTimer) clearTimeout(this.autosaveTimer)
    this.autosaveTimer = null
  }

  /** Resolves only after all edits present while earlier saves ran have settled. */
  flush(): Promise<boolean> {
    this.clearAutosave()
    if (this.saveFlight) return this.saveFlight
    const flight = this.saveUntilSettled().then((result) => {
      if (this.saveFlight === flight) this.saveFlight = null
      return result
    })
    this.saveFlight = flight
    return flight
  }

  private async saveUntilSettled(): Promise<boolean> {
    let retryCount = 0
    while (this.editor && this.version) {
      if (this.editor.conflicts.length) {
        this.publish({ saveError: this.conflictError() })
        return false
      }
      const submitted = this.editor.draft
      if (stableStringify(submitted) === stableStringify(this.editor.acknowledgedDraft)) return true
      const expectedVersion = this.version
      this.publish({ saveError: '' })
      try {
        const saved = await this.port.save(submitted, expectedVersion)
        const latest = this.editor?.draft ?? submitted
        this.version = saved.version
        this.editor = acceptSettingsSave(submitted, saved.settings, latest)
        this.publish({ saveError: this.conflictError() })
        this.onSettingsChange()
        retryCount = 0
      } catch (error) {
        let failure = error
        if (isSettingsVersionConflict(error) && retryCount < 3) {
          retryCount += 1
          try {
            const fresh = await this.port.refresh()
            this.version = fresh.version
            this.editor = receiveSettingsSnapshot(this.editor, fresh.settings)
            this.publish({ saveError: this.conflictError() })
            if (!this.editor.conflicts.length) continue
            return false
          } catch (refreshError) {
            failure = refreshError
          }
        }
        const message = failure instanceof Error ? failure.message : String(failure)
        this.publish({ saveError: `Save failed: ${message.replace(/\n/g, ' ')}` })
        return false
      } finally {
        const pending = this.pendingSnapshot
        this.pendingSnapshot = null
        if (pending) this.acceptDirect(pending)
      }
    }
    return false
  }

  private acceptDirect(snapshot: SettingsSnapshot) {
    if (this.version?.epoch === snapshot.version.epoch && this.version.revision >= snapshot.version.revision) return
    this.version = snapshot.version
    this.editor = this.editor
      ? receiveSettingsSnapshot(this.editor, snapshot.settings)
      : createSettingsEditorState(snapshot.settings)
    this.publish({ saveError: this.conflictError() })
  }

  /** Replacement after an explicit import or a separately owned narrow write. */
  replace(snapshot: SettingsSnapshot) {
    this.version = snapshot.version
    this.editor = createSettingsEditorState(snapshot.settings)
    this.publish({ loading: false, loadError: '', saveError: '' })
  }

  async import(path: string): Promise<void> {
    if (!this.port.import) throw new Error('Settings import is unavailable')
    if (!await this.flush()) throw new Error(this.view.saveError || 'Save failed: unsaved settings remain')
    if (!this.version) throw new Error('Settings version is unavailable')
    const imported = await this.port.import(path, this.version)
    this.replace(imported)
    this.onSettingsChange()
  }

  /** Keeps the legacy synchronous close callback contract at the page seam. */
  requestClose(onClose: () => void, options?: SettingsCloseOptions): Promise<void> {
    const pending = this.flush()
    const generation = this.loadGeneration
    const closeOnce = () => {
      if (this.disposed || generation !== this.loadGeneration) return
      onClose()
    }
    if (options?.waitForSave === false) {
      // Navigation supersedes any ordinary close still waiting for this save.
      this.closeFlight = null
      closeOnce()
      return pending.then(() => {})
    }
    if (this.closeFlight) return this.closeFlight
    const flight = pending.then((saved) => {
      // Deduplicate only this pending request, not future keep-alive visits.
      if (this.closeFlight !== flight) return
      // A failed/conflicted draft stays visible for repair and retry. Loading
      // or read-error pages have no editable draft and may always close.
      if (saved || !this.editor) closeOnce()
    }).finally(() => {
      if (this.closeFlight === flight) this.closeFlight = null
    })
    this.closeFlight = flight
    return flight
  }

  dispose() {
    this.disposed = true
    this.loadGeneration += 1
    this.unsubscribe?.()
    this.unsubscribe = null
    this.clearAutosave()
    this.listeners.clear()
  }
}
