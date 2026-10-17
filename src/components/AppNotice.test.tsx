import { StrictMode } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AppNoticeHost, ErrorNotice, OperationNotice } from './AppNotice'
import { dismissNotice, getNotices, notifyError } from './appNoticeStore'

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  getNotices().forEach(notice => dismissNotice(notice.id))
})

describe('shared operation errors', () => {
  it('bounds concurrent errors, deduplicates repeated failures, keeps full details and allows dismissal', () => {
    render(<AppNoticeHost />)
    const longError = '备份上传失败：' + '网络连接中断，请稍后重试。'.repeat(20)
    act(() => {
      notifyError('较早失败')
      notifyError('保存失败')
      notifyError('读取失败')
      notifyError(longError)
      notifyError(longError)
    })
    expect(screen.getAllByRole('alert')).toHaveLength(3)
    expect(screen.queryByText('较早失败')).not.toBeInTheDocument()
    expect(screen.getAllByRole('alert')[2].textContent!.length).toBeLessThanOrEqual(101)
    fireEvent.click(screen.getByText('查看详情'))
    expect(screen.getByText(longError)).toBeVisible()
    fireEvent.click(screen.getAllByRole('button', { name: '关闭提示' })[2])
    expect(screen.queryByText(longError)).not.toBeInTheDocument()
  })

  it('follows page error state without reopening dismissed errors on ordinary renders or leaving stale errors', () => {
    const Page = ({ message }: { message: string }) => <StrictMode><AppNoticeHost /><ErrorNotice message={message} /></StrictMode>
    const view = render(<Page message="保存失败" />)
    expect(screen.getAllByRole('alert')).toHaveLength(1)
    fireEvent.click(screen.getByRole('button', { name: '关闭提示' }))
    view.rerender(<Page message="保存失败" />)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    view.rerender(<Page message="网络中断" />)
    expect(screen.getByRole('alert')).toHaveTextContent('网络中断')
    view.rerender(<Page message="" />)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    view.rerender(<Page message="再次失败" />)
    view.unmount()
    render(<AppNoticeHost />)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })
  it('expires success feedback while keeping a retryable error and invokes the latest retry action', () => {
    vi.useFakeTimers()
    const firstRetry = vi.fn()
    const latestRetry = vi.fn()
    const Page = ({ retry }: { retry: () => void }) => <>
      <main data-testid="page"><OperationNotice message="已导出 RIS" tone="success" duration={3500} />
        <ErrorNotice message="保存失败" actionLabel="重试保存" onAction={retry} /></main>
      <AppNoticeHost />
    </>
    const view = render(<Page retry={firstRetry} />)
    expect(screen.getByTestId('page')).toBeEmptyDOMElement()
    expect(screen.getByRole('status')).toHaveTextContent('已导出 RIS')
    view.rerender(<Page retry={latestRetry} />)
    fireEvent.click(screen.getByRole('button', { name: '重试保存' }))
    expect(firstRetry).not.toHaveBeenCalled()
    expect(latestRetry).toHaveBeenCalledTimes(1)
    act(() => { vi.advanceTimersByTime(3500) })
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('保存失败')
  })

})
