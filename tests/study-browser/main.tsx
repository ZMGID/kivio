import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { StudyWorkspace } from '../../src/chat/study/StudyWorkspace'
import { AppDialogHost } from '../../src/components/AppDialog'
import { Button } from '../../src/components/Button'
import { LangContext } from '../../src/components/i18n'
import '../../src/styles/app.css'
import 'streamdown/styles.css'

export function Fixture() {
  const [visible, setVisible] = useState(true)
  return <LangContext.Provider value="en"><div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '5px 12px', fontSize: 11, background: 'var(--theme-surface-soft)', color: 'var(--text-muted)' }}>
      <span>BROWSER TEST ONLY · Simulated provider · No model calls</span>
      <Button size="sm" onClick={() => setVisible((value) => !value)}>{visible ? 'Leave Study' : 'Return to Study'}</Button>
      <Button size="sm" onClick={() => document.documentElement.classList.toggle('dark')}>Toggle theme</Button>
    </div>
    {visible ? <StudyWorkspace onOpenSettings={() => { document.body.dataset.settingsOpened = 'true' }} /> : <p>Another page</p>}
    <AppDialogHost />
  </div></LangContext.Provider>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
