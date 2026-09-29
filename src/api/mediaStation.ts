import { invoke } from '@tauri-apps/api/core'
import type { MediaJob, MediaRequest } from '../generated/mediaStation'
export type { MediaJob, MediaRequest, MediaKind } from '../generated/mediaStation'

export const mediaStationApi = {
  list: () => invoke<MediaJob[]>('media_station_list'),
  start: (request: MediaRequest) => invoke<MediaJob>('media_station_start', { request }),
  cancel: (id: string) => invoke<void>('media_station_cancel', { id }),
  reference: (id: string, index: number) => invoke<string>('media_station_reference', { id, index }),
  read: (id: string, index: number) => invoke<ArrayBuffer | number[]>('media_station_read', { id, index }),
  export: (id: string, index: number, destination: string) => invoke<void>('media_station_export', { id, index, destination }),
}
