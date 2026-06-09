import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

const dirname = path.dirname(fileURLToPath(import.meta.url))
const pyodideDir = path.resolve(dirname, 'node_modules/pyodide')
const pythonSandboxPyodideDir = path.resolve(dirname, 'resources/python-sandbox/pyodide')
const pyodideCoreAssetFiles = [
  'pyodide.asm.js',
  'pyodide.asm.wasm',
  'pyodide-lock.json',
  'python_stdlib.zip',
]
const pyodideManifestAssetFiles = [
  'pyodide-package-manifest.json',
]

function contentTypeForPyodideAsset(fileName: string): string {
  if (fileName.endsWith('.wasm')) return 'application/wasm'
  if (fileName.endsWith('.zip') || fileName.endsWith('.whl')) return 'application/zip'
  if (fileName.endsWith('.tar')) return 'application/x-tar'
  if (fileName.endsWith('.otf')) return 'font/otf'
  if (fileName.endsWith('.ttf')) return 'font/ttf'
  if (fileName.endsWith('.json')) return 'application/json; charset=utf-8'
  return 'text/javascript; charset=utf-8'
}

function pyodideAssetSourceDir(): string {
  if (
    fs.existsSync(pythonSandboxPyodideDir)
    && pyodideCoreAssetFiles.every((fileName) => fs.existsSync(path.join(pythonSandboxPyodideDir, fileName)))
  ) {
    return pythonSandboxPyodideDir
  }
  return pyodideDir
}

function listPyodideAssetFiles(sourceDir: string): string[] {
  if (!fs.existsSync(sourceDir)) return []
  const files = fs.readdirSync(sourceDir, { withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
    .filter((fileName) => (
      pyodideCoreAssetFiles.includes(fileName)
      || pyodideManifestAssetFiles.includes(fileName)
      || fileName.endsWith('.whl')
      || fileName.endsWith('.zip')
      || fileName.endsWith('.tar')
      || fileName.endsWith('.otf')
      || fileName.endsWith('.ttf')
    ))
    .sort()
  if (sourceDir === pyodideDir) {
    return files.filter((fileName) => pyodideCoreAssetFiles.includes(fileName))
  }
  return files
}

function pyodideAssetsPlugin(): Plugin {
  return {
    name: 'kivio-pyodide-assets',
    configureServer(server) {
      server.middlewares.use('/pyodide/', (req, res, next) => {
        const fileName = path.basename(decodeURIComponent((req.url ?? '').split('?')[0]))
        const sourceDir = pyodideAssetSourceDir()
        const assetFiles = listPyodideAssetFiles(sourceDir)
        if (!assetFiles.includes(fileName)) {
          res.statusCode = 404
          res.end(`Pyodide asset not found: ${fileName}`)
          return
        }
        const filePath = path.join(sourceDir, fileName)
        res.setHeader('Content-Type', contentTypeForPyodideAsset(fileName))
        fs.createReadStream(filePath)
          .on('error', next)
          .pipe(res)
      })
    },
    generateBundle() {
      const sourceDir = pyodideAssetSourceDir()
      for (const fileName of listPyodideAssetFiles(sourceDir)) {
        this.emitFile({
          type: 'asset',
          fileName: `pyodide/${fileName}`,
          source: fs.readFileSync(path.join(sourceDir, fileName)),
        })
      }
    },
  }
}

// https://vitejs.dev/config/
// Vite 构建配置：使用 @vitejs/plugin-react 插件支持 React 开发
export default defineConfig({
  base: './',
  plugins: [react(), pyodideAssetsPlugin()],
  build: {
    target: 'esnext',
  },
})
