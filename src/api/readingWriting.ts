import { invoke } from '@tauri-apps/api/core'
export type WordConnection = { running: boolean; installed: boolean; pairingCode: string }
export const readingWritingApi = {
  status: () => invoke<WordConnection>('reading_word_status'),
  start: () => invoke<WordConnection>('reading_word_start'),
  stop: () => invoke<WordConnection>('reading_word_stop'),
  install: () => invoke<WordConnection>('reading_word_install'),
  exportBib: (ids: string[], path: string) => invoke<void>('reading_export_bib', { ids, path }),
}
