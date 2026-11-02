import { describe, expect, it } from 'vitest'
import { FREEROUTE_BASE_URL, isFreeRoute, singleVendorApiKey } from './tauri'

describe('FreeRoute helpers', () => {
  it('recognizes the aggregator endpoint', () => {
    expect(isFreeRoute({ baseUrl: FREEROUTE_BASE_URL })).toBe(true)
    expect(isFreeRoute({ baseUrl: `${FREEROUTE_BASE_URL}/` })).toBe(true)
    expect(isFreeRoute({ baseUrl: 'https://openrouter.ai/api/v1' })).toBe(false)
  })

  it('stores exactly one key per vendor from a multi-line paste', () => {
    expect(singleVendorApiKey('  sk-one \n sk-two \n')).toBe('sk-one')
    expect(singleVendorApiKey('sk-one,sk-two')).toBe('sk-one')
    expect(singleVendorApiKey('\n\n')).toBe('')
  })
})
