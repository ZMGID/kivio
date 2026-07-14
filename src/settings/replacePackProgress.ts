import type { OfflineModelProgress } from '../api/tauri'

export type ReplacePackProgressState = {
  downloadState: 'idle' | 'downloading' | 'failed'
  error: string
  progress: OfflineModelProgress | null
}

export type ReplacePackProgressAction =
  | { type: 'start' }
  | { type: 'progress'; progress: OfflineModelProgress }
  | { type: 'success' }
  | { type: 'failure'; error: string }

export const initialReplacePackProgressState: ReplacePackProgressState = {
  downloadState: 'idle',
  error: '',
  progress: null,
}

export function reduceReplacePackProgress(
  state: ReplacePackProgressState,
  action: ReplacePackProgressAction,
): ReplacePackProgressState {
  switch (action.type) {
    case 'start':
      return { downloadState: 'downloading', error: '', progress: null }
    case 'progress': {
      if (action.progress.state === 'failed') {
        return {
          downloadState: 'failed',
          error: action.progress.error || state.error,
          progress: action.progress,
        }
      }
      // 终态：整包字节跑满的 completed（最后一个文件校验完成）回到 idle。
      // completed 事件是逐文件发的，中途的 completed 仍视为下载中，
      // 否则面板会在文件间隙闪烁；没有终态则 install promise 丢失时永远卡在“下载中”。
      if (
        action.progress.state === 'completed'
        && action.progress.overallDownloadedBytes >= action.progress.overallTotalBytes
      ) {
        return { downloadState: 'idle', error: '', progress: action.progress }
      }
      return { ...state, downloadState: 'downloading', progress: action.progress }
    }
    case 'success':
      return { ...state, downloadState: 'idle', error: '' }
    case 'failure':
      return { ...state, downloadState: 'failed', error: action.error }
  }
}
