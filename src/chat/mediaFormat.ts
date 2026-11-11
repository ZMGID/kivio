// 媒体站的纯格式化函数：比例解析与按日分组的时间标签。

export function ratioOf(aspectRatio: string) {
  const [w, h] = aspectRatio.split(':').map(Number)
  return w > 0 && h > 0 ? w / h : 1
}

export function dayLabel(timestamp: number, zh: boolean) {
  const now = new Date()
  const date = new Date(timestamp)
  const startOf = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
  const days = Math.round((startOf(now) - startOf(date)) / 86_400_000)
  if (days === 0) return zh ? '今天' : 'Today'
  if (days === 1) return zh ? '昨天' : 'Yesterday'
  return date.toLocaleDateString(zh ? 'zh-CN' : 'en-US', { month: 'long', day: 'numeric', ...(date.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }) })
}

export function timeLabel(timestamp: number, zh: boolean) {
  return new Date(timestamp).toLocaleTimeString(zh ? 'zh-CN' : 'en-US', { hour: '2-digit', minute: '2-digit' })
}

/** Comma-separated clauses of a prompt, without its closing full stop. */
function promptTerms(prompt: string) {
  return prompt.trim().replace(/[。.]+$/, '').split(/[，,]/).map((part) => part.trim()).filter(Boolean)
}

export function promptHasTerm(prompt: string, term: string) {
  return promptTerms(prompt).includes(term)
}

/** Adds the term as a new clause, or removes that exact clause; other wording is kept as written. */
export function togglePromptTerm(prompt: string, term: string, zh: boolean) {
  const terms = promptTerms(prompt)
  const next = terms.includes(term) ? terms.filter((part) => part !== term) : [...terms, term]
  if (!next.length) return ''
  return zh ? `${next.join('，')}。` : `${next.join(', ')}.`
}
