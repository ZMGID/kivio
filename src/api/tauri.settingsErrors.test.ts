import { describe, expect, it } from 'vitest'
import { isSettingsVersionConflict } from './tauri'

const conflict = {
  code: 'versionConflict',
  message: 'stale settings',
  expectedVersion: { epoch: 'a', revision: 1 },
  actualVersion: { epoch: 'a', revision: 2 },
}

describe('isSettingsVersionConflict', () => {
  it('recognizes structured Tauri command errors', () => {
    expect(isSettingsVersionConflict(conflict)).toBe(true)
  })

  it('recognizes JSON error strings used by some Tauri rejection adapters', () => {
    expect(isSettingsVersionConflict(JSON.stringify(conflict))).toBe(true)
    expect(isSettingsVersionConflict(new Error(JSON.stringify(conflict)))).toBe(true)
  })

  it('does not misclassify operational failures', () => {
    expect(isSettingsVersionConflict({ code: 'operationFailed', message: 'disk full' })).toBe(false)
    expect(isSettingsVersionConflict(new Error('disk full'))).toBe(false)
  })
})
