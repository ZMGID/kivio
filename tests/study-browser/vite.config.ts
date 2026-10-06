import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'

const fixture = (name: string) => fileURLToPath(new URL(name, import.meta.url))
// Only the browser-test server replaces external I/O. The production Vite config
// never imports this file and the app build contains no fixture transport.
export default defineConfig({
  plugins: [{ name: 'study-browser-fixtures', enforce: 'pre', resolveId(source, importer) {
    if (importer?.endsWith('/studyRequest.ts') && source === '../../api/study') return fixture('./mockTransport.ts')
    if (importer?.endsWith('/StudyWorkspace.tsx') && source === '../../api/settingsCache') return fixture('./mockSettings.ts')
    return null
  } }, react()],
  server: { host: '127.0.0.1', port: 5714, strictPort: true },
})
