export function isExecutableAgentPlanText(content?: string | null): boolean {
  const text = content?.trim() ?? ''
  if (!text) return false

  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
  if (lines.length < 2) return false

  const stepLines = lines.filter(isStepLikeLine).length
  if (stepLines >= 2) return true

  return hasPlanKeyword(text) && stepLines >= 1
}

function isStepLikeLine(line: string): boolean {
  return isMarkdownStep(line)
    || isChineseStep(line)
    || isTodoKeywordStep(line)
}

function isMarkdownStep(line: string): boolean {
  return /^[-*+] \[[ xX]\]\s+/.test(line)
    || /^[-*+•]\s+/.test(line)
    || /^\d{1,3}[.)、]\s*/.test(line)
}

function isChineseStep(line: string): boolean {
  return /^(第[一二三四五六七八九十\d]+步|步驟[一二三四五六七八九十\d]+|[一二三四五六七八九十]、)/.test(line)
}

function isTodoKeywordStep(line: string): boolean {
  return /^(todo[:\s]|step\s|步驟[:：]|任務[:：])/i.test(line)
}

function hasPlanKeyword(text: string): boolean {
  return /plan|todo|step/i.test(text)
    || /計畫|步驟|待辦|任務/.test(text)
}
