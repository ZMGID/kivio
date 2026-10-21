import { invoke } from '@tauri-apps/api/core'
import type { ReadingNote } from './reading'

export type ReadingTranslation = {
  runId: string
  phase: 'running' | 'ready' | 'error' | 'cancelled'
  percent: number
  stage: string
  model: string
  targetLang: string
  error: string | null
  page?: number | null
}
export type ReadingPdfKind = 'source' | 'mono' | 'dual'
export type ReadingOutput = { runId: string; targetLang: string; page: number | null }
export type ReadingVersion = ReadingOutput & { createdAt: number }
export type ReadingView = { kind: ReadingPdfKind; output: ReadingOutput | null; latest: ReadingOutput | null }
export type ReadingSession = { translation: ReadingTranslation | null; view: ReadingView }
export type ReadingExportFormat = 'source' | 'mono' | 'dual' | 'md' | 'docx' | 'epub'
export const readingOperationsApi = {
  versions: (id: string) => invoke<ReadingVersion[]>('reading_versions', { id }),
  status: (id: string) => invoke<ReadingSession>('reading_translation_status', { id }),
  translate: (id: string, providerId: string, model: string, sourceLang: string, targetLang: string, page: number | null) =>
    invoke<ReadingSession>('reading_translation_start', { id, providerId, model, sourceLang, targetLang, page }),
  selectView: (id: string, kind: ReadingPdfKind, runId?: string) => invoke<ReadingSession>('reading_view_select', { id, kind, runId }),
  cancel: (id: string) => invoke<void>('reading_translation_cancel', { id }),
  export: (id: string, path: string, format: ReadingExportFormat, notes: ReadingNote[], draft: string, runId?: string) =>
    invoke<void>('reading_export', { id, path, format, notes, draft, runId }),
}
