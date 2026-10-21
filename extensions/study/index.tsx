import { defineExtension } from '../sdk'
import ReadingCenter from './ui/ReadingCenter'
import { flushReadingLibrary } from './ui/readingLibrary'

const study = defineExtension({
  Page: ReadingCenter,
  activate(api) {
    api.on('beforeDisable', async () => {
      try { await flushReadingLibrary() }
      catch (error) { return { cancel: true, reason: `Study 尚未保存，无法关闭：${error instanceof Error ? error.message : String(error)}` } }
    })
  },
})

export default study
