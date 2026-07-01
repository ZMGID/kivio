import { readFileSync, writeFileSync } from 'fs'
import { Converter } from 'opencc-js'

const converter = Converter({ from: 'cn', to: 'tw' })

const files = [
  'src/settings/SettingsShell.tsx',
  'src/settings/KivioCodeSettings.tsx',
  'src/settings/RetrievalPanel.tsx',
  'src/settings/DocumentProcessingPanel.tsx',
  'src/settings/KnowledgeBasePanel.tsx',
  'src/settings/UsageStatsPanel.tsx',
  'src/settings/ProviderModelsPicker.tsx',
  'src/settings/ProviderSortableList.tsx',
]

for (const file of files) {
  const content = readFileSync(file, 'utf-8')
  // Convert ONLY CJK characters inside string literals (between quotes)
  const result = content.replace(
    /('(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|`(?:[^`\\]|\\.)*`)/g,
    (match) => {
      // Only convert if the string contains CJK characters
      if (/[\u4e00-\u9fff]/.test(match)) {
        return converter(match)
      }
      return match
    }
  )
  writeFileSync(file, result, 'utf-8')
  console.log(`Converted: ${file}`)
}
