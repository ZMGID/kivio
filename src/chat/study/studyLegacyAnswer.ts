/** Read-only compatibility for replies saved during the earlier structured-output experiment.
 * New requests do not ask for this format; unrecognized replies remain ordinary model text.
 */
export function readLegacyStudyAnswer(answer: string): { visibleText: string; withheldSolution?: string } | undefined {
  let value: Record<string, unknown>
  const trimmed = answer.trim()
  const fence = /^```(?:json)?[ \t]*\r?\n([\s\S]*?)\r?\n```$/i.exec(trimmed)
  try { value = JSON.parse(fence ? fence[1] : trimmed) } catch { return undefined }
  if (!value || typeof value !== 'object' || value.version !== 1) return undefined
  const fields = value.mode === 'hint' ? ['hint'] : value.mode === 'check' ? ['firstIssue', 'nextStep', 'verification'] : []
  if (!fields.length || fields.some(field => typeof value[field] !== 'string')) return undefined
  return { visibleText: fields.map(field => value[field] as string).join('\n\n'), withheldSolution: typeof value.withheldSolution === 'string' ? value.withheldSolution : undefined }
}
