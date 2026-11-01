import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { ProviderModelTestModal } from './ProviderModelTestModal'
import { MODEL_TEST_CONCURRENCY } from '../components/providerModelTestPool'

vi.mock('../api/tauri', () => ({
  api: {
    testProviderConnection: vi.fn(),
  },
}))

vi.mock('../components/ModelIcon', () => ({
  ModelIcon: () => null,
}))

describe('ProviderModelTestModal', () => {
  it('does not fire every selected model at once', async () => {
    const user = userEvent.setup()
    let inflight = 0
    let peak = 0
    vi.mocked(api.testProviderConnection).mockImplementation(async () => {
      inflight += 1
      peak = Math.max(peak, inflight)
      await new Promise((r) => setTimeout(r, 20))
      inflight -= 1
      return { success: true }
    })

    const models = Array.from({ length: 14 }, (_, i) => `m${i}`)
    render(
      <ProviderModelTestModal
        providerId="p1"
        baseUrl="https://example.com/v1"
        apiKeys={['sk-test']}
        apiFormat="openai_chat"
        models={models}
        lang="zh"
        onClose={() => {}}
      />,
    )

    await user.click(screen.getByRole('button', { name: /开始测试/ }))
    await waitFor(() => {
      expect(api.testProviderConnection).toHaveBeenCalledTimes(14)
    })
    expect(peak).toBeLessThanOrEqual(MODEL_TEST_CONCURRENCY)
    expect(peak).toBeGreaterThan(1)
  })

})
