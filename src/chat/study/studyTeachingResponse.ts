export type StudyTeachingMode = 'hint' | 'check'

/** Bounds are UTF-16 string lengths, shared by prompt assembly and response validation. */
export const STUDY_TEACHING_RESPONSE_LIMITS = {
  visibleField: 800,
  withheldSolution: 12_000,
  total: 20_000,
} as const

export interface StudyHintResponse {
  version: 1
  mode: 'hint'
  hint: string
  withheldSolution?: string
}

export interface StudyCheckResponse {
  version: 1
  mode: 'check'
  verification: string
  firstIssue: string
  nextStep: string
  withheldSolution?: string
}

export type StudyTeachingResponse = StudyHintResponse | StudyCheckResponse
export type StudyTeachingResponseResult =
  | { status: 'valid'; response: StudyTeachingResponse }
  | { status: 'fallback'; reason: 'invalid-json' | 'invalid-shape' | 'mode-mismatch' | 'too-long' }

function isBoundedText(value: unknown, limit: number): value is string {
  return typeof value === 'string' && value.trim().length > 0 && value.length <= limit
}

/** JSON.parse otherwise silently accepts the last of repeated object keys. */
function hasDuplicateKeys(json: string): boolean {
  const keys = new Set<string>()
  // Called only for a validated flat object of primitive values. String tokens consume
  // escaped quotes, so key-like text inside a teaching field cannot count as a key.
  for (const token of json.matchAll(/"(?:\\.|[^"\\])*"\s*(:)?/g)) {
    if (!token[1]) continue
    const key: string = JSON.parse(token[0].slice(0, token[0].lastIndexOf(':')))
    if (keys.has(key)) return true
    keys.add(key)
  }
  return false
}

/**
 * Validates the complete response envelope, never a streamed prefix or guessed prose section.
 * A valid envelope does not establish mathematical accuracy or teaching-mode compliance.
 * Callers must keep withheldSolution hidden until an explicit reveal, and treat fallback as
 * unstructured model output rather than automatically displaying it as teaching feedback.
 */
export function parseStudyTeachingResponse(content: unknown, expectedMode: StudyTeachingMode): StudyTeachingResponseResult {
  if (typeof content !== 'string') return { status: 'fallback', reason: 'invalid-json' }
  if (content.length > STUDY_TEACHING_RESPONSE_LIMITS.total) return { status: 'fallback', reason: 'too-long' }

  const trimmed = content.trim()
  // Tolerate one outer JSON fence only; no surrounding prose or multiple candidate objects.
  const fence = /^```(?:json)?[ \t]*\r?\n([\s\S]*?)\r?\n```$/i.exec(trimmed)
  const json = fence ? fence[1].trim() : trimmed
  let parsed: unknown
  try { parsed = JSON.parse(json) }
  catch { return { status: 'fallback', reason: 'invalid-json' } }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return { status: 'fallback', reason: 'invalid-shape' }

  const value = parsed as Record<string, unknown>
  if (value.version !== 1) return { status: 'fallback', reason: 'invalid-shape' }
  if ((expectedMode !== 'hint' && expectedMode !== 'check') || value.mode !== expectedMode) {
    return { status: 'fallback', reason: 'mode-mismatch' }
  }
  const requiredFields = expectedMode === 'hint' ? ['hint'] : ['verification', 'firstIssue', 'nextStep']
  const requiredKeys = ['version', 'mode', ...requiredFields]
  const allowedKeys = [...requiredKeys, 'withheldSolution']
  const keys = Object.keys(value)
  if (keys.some(key => !allowedKeys.includes(key)) || requiredKeys.some(key => !keys.includes(key))
    || requiredFields.some(key => !isBoundedText(value[key], STUDY_TEACHING_RESPONSE_LIMITS.visibleField))
    || (keys.includes('withheldSolution') && !isBoundedText(value.withheldSolution, STUDY_TEACHING_RESPONSE_LIMITS.withheldSolution))) {
    return { status: 'fallback', reason: 'invalid-shape' }
  }
  if (hasDuplicateKeys(json)) return { status: 'fallback', reason: 'invalid-shape' }

  // The closed schema and field types above are the authority for this type assertion.
  return { status: 'valid', response: value as unknown as StudyTeachingResponse }
}
