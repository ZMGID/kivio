import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.tsx'
import './styles/app.css'
import 'streamdown/styles.css'
import { isTauriRuntime } from './api/tauri'
import { startBackendSettingsSync } from './api/settingsCache'
import { startModelDatabaseSync } from './data/modelDatabaseSync'
import { restoreBackupInterface } from './api/backupInterface'

if (isTauriRuntime()) {
  const backendSettingsSync = startBackendSettingsSync().catch((error) => {
    console.error('[settingsCache] could not subscribe to backend updates', error)
    return () => {}
  })
  if (import.meta.hot) import.meta.hot.dispose(() => { void backendSettingsSync.then((stop) => stop()) })
}

// 屏蔽 WebView 原生右键菜单（Back/Reload/Inspect）
document.addEventListener('contextmenu', (e) => e.preventDefault())

// React 应用入口文件
// 使用 createRoot 将 App 组件挂载到 DOM 的 #root 元素上
function mountApp() {
  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  )
}

if (isTauriRuntime()) {
  void restoreBackupInterface().then(() => startModelDatabaseSync()).then((stop) => {
    if (import.meta.hot) import.meta.hot.dispose(stop)
    mountApp()
  }).catch((error) => {
    console.error('[startup] could not load local data', error)
    // Do not silently show bundled metadata while the backend uses a newer catalog.
    const root = document.getElementById('root')!
    root.textContent = '本地数据加载失败，请重新打开窗口。 / Local data failed to load. Reopen this window.'
  })
} else {
  mountApp()
}
