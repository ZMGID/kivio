import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vitejs.dev/config/
// Vite 构建配置：使用 @vitejs/plugin-react 插件支持 React 开发
export default defineConfig({
  base: './',
  plugins: [react()],
  build: {
    target: 'esnext',
  },
})
