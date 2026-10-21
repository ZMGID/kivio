import { build } from 'vite'
import react from '@vitejs/plugin-react'
import { copyFile } from 'node:fs/promises'
import { resolve } from 'node:path'
await build({ configFile: false, root: resolve('extensions/study/word'), base: '/', plugins: [react()], build: { outDir: resolve('src-tauri/resources/word'), emptyOutDir: true } })
await copyFile('public/icon.png', 'src-tauri/resources/word/icon.png')
