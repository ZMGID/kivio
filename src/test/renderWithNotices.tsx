import { type ReactNode } from 'react'
import { render as renderReact } from '@testing-library/react'
import { afterEach } from 'vitest'
import { AppNoticeHost } from '../components/AppNotice'
import { dismissNotice, getNotices } from '../components/appNoticeStore'

// Exercise operation feedback with the same shared host as the desktop shell.
export function render(ui: ReactNode) {
  return renderReact(ui, { wrapper: ({ children }) => <><AppNoticeHost />{children}</> })
}
afterEach(() => getNotices().forEach(notice => dismissNotice(notice.id)))
