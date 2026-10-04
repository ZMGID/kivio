import { describe, expect, it } from 'vitest'
import { isChatOnboardingPath } from './persistence'

describe('chat onboarding routes', () => {
  it('recognizes onboarding path', () => {
    expect(isChatOnboardingPath('chat/onboarding')).toBe(true)
    expect(isChatOnboardingPath('chat/onboarding/step')).toBe(true)
    expect(isChatOnboardingPath('chat/settings')).toBe(false)
  })

})
