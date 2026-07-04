import { readFileSync, readdirSync, statSync, writeFileSync } from 'fs'
import path from 'path'
import { Converter } from 'opencc-js'

const converter = Converter({ from: 'cn', to: 'twp' })

const projectTerminology = [
  ['檢檢視表', '檢視圖表'],
  ['檢檢視', '檢視'],
  ['聯網搜尋', '網路搜尋'],
  ['聯網', '網路'],
  ['聯結器', '連接器'],
  ['聯結', '連接'],
  ['服務商', '供應商'],
  ['提供商', '供應商'],
  ['自定義', '自訂'],
  ['獲取', '取得'],
  ['檢測', '偵測'],
  ['文本', '文字'],
  ['響應', '回應'],
  ['窗口', '視窗'],
  ['接口', '介面'],
  ['密鑰', '金鑰'],
  ['本地', '本機'],
  ['全局', '全域'],
  ['全域性', '全域'],
  ['全屏', '全螢幕'],
  ['剪貼板', '剪貼簿'],
  ['認證方式', '驗證方式'],
  ['認證', '驗證'],
  ['概覽', '總覽'],
  ['協議', '協定'],
  ['上報', '回報'],
  ['計劃', '計畫'],
  ['高級', '進階'],
  ['構建', '建置'],
  ['模板', '範本'],
  ['條已壓縮', '則已壓縮'],
]

const roots = [
  'src/App.tsx',
  'src/Lens.tsx',
  'src/chat',
  'src/components',
  'src/onboarding',
  'src/settings',
]

const ignoredFiles = new Set([
  // Keep the Simplified Chinese locale table and its assertions intact.
  'src/settings/i18n.ts',
  'src/settings/i18n.test.ts',
  // This catalog intentionally carries parallel zh / zh-TW fields.
  'src/settings/connectorCatalog.ts',
])

function collectFiles(entry) {
  const stat = statSync(entry)
  if (stat.isFile()) {
    return /\.(ts|tsx|js|jsx|css)$/.test(entry) && !ignoredFiles.has(entry) ? [entry] : []
  }

  return readdirSync(entry).flatMap((name) => collectFiles(path.join(entry, name)))
}

const files = [...new Set(roots.flatMap(collectFiles))].sort()

for (const file of files) {
  const content = readFileSync(file, 'utf-8')
  if (!/[\u4e00-\u9fff]/.test(content)) continue

  let result = converter(content)
  for (const [from, to] of projectTerminology) {
    result = result.replaceAll(from, to)
  }
  if (result === content) continue

  writeFileSync(file, result, 'utf-8')
  console.log(`Converted: ${file}`)
}
