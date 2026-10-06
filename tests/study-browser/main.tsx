import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { StudyWorkspace } from '../../src/chat/study/StudyWorkspace'
import { AppDialogHost } from '../../src/components/AppDialog'
import { Button } from '../../src/components/Button'
import { applyThemeSettings } from '../../src/theme/theme'
import { LangContext } from '../../src/components/i18n'
import '../../src/styles/app.css'
import 'streamdown/styles.css'

applyThemeSettings({ theme: 'light', themeColor: 'neutral', customThemes: [] })

export function Fixture() {
  const [visible, setVisible] = useState(true)
  return <LangContext.Provider value={new URLSearchParams(window.location.search).get('lang') === 'zh' ? 'zh' : 'en'}><div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '5px 12px', fontSize: 11, background: 'var(--theme-surface-soft)', color: 'var(--text-muted)' }}>
      <span>BROWSER TEST ONLY · Simulated provider · No model calls</span>
      <Button size="sm" onClick={() => setVisible((value) => !value)}>{visible ? 'Leave Study' : 'Return to Study'}</Button>
      <Button size="sm" onClick={() => applyThemeSettings({ theme: document.documentElement.classList.contains('dark') ? 'light' : 'dark', themeColor: 'neutral', customThemes: [] })}>Toggle theme</Button>
    </div>
    {visible ? <StudyWorkspace onOpenSettings={() => { document.body.dataset.settingsOpened = 'true' }} /> : <p>Another page</p>}
    <AppDialogHost />
  </div></LangContext.Provider>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
