import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'
const fixture = (name: string) => fileURLToPath(new URL(name, import.meta.url))
export default defineConfig({
  plugins: [{ name: 'study-browser-attachment-read', enforce: 'pre', resolveId(source, importer) {
    if (source === './attachmentPreview' && importer?.includes('/src/chat/')) return fixture('./mockAttachmentPreview.ts')
    return null
  } }, react()],
  build: { target: 'esnext', rollupOptions: { input: fixture('./index.html') } },
  server: { host: '127.0.0.1', port: 5714, strictPort: true },
})
