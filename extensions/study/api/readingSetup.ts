import { invoke } from '@tauri-apps/api/core'

export type StudySetupProgress = {
  component: 'pdf2zh' | 'pandoc'
  steps: number
  phase: 'extract' | 'download' | 'python' | 'packages' | 'assets' | 'verify' | 'ready' | 'error'
  step: number
  downloaded: number
  total: number | null
  error: string | null
}
export type StudySetupStatus = {
  acknowledged: boolean
  pdf2zh: boolean
  installed: boolean
  version: string
  pandoc: boolean
  pandocInstalled: boolean
  pandocVersion: string
  pandocDownloadBytes: number | null
  progress: StudySetupProgress | null
}
export const readingSetupApi = {
  status: () => invoke<StudySetupStatus>('reading_setup_status'),
  save: (selection: { pdf2zh: boolean; pandoc: boolean }, acknowledge = false) =>
    invoke<StudySetupStatus>('reading_setup_save', { ...selection, acknowledge }),
  start: (repair = false) => invoke<StudySetupStatus>('reading_setup_start', { repair }),
}
