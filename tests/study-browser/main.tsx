import { createRoot } from 'react-dom/client'
import Chat from '../../src/chat/Chat'
import { Button } from '../../src/components/Button'
import { applyThemeSettings } from '../../src/theme/theme'
import { installChatFixture } from './mockTransport'
import { installSettingsFixture } from './mockSettings'
import '../../src/styles/app.css'
import 'streamdown/styles.css'

// The production Chat root owns navigation, drafts, execution, message rendering,
// cancellation and retry. Only external I/O is simulated in this fixture.
installSettingsFixture()
installChatFixture()
if (!location.hash) location.hash = '#chat/study'
localStorage.setItem('kivio-chat-sidebar-collapsed', '1')
applyThemeSettings({ theme: 'light', themeColor: 'neutral', customThemes: [] })

export function Fixture() {
  return <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
    <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: 8, padding: '4px 12px', fontSize: 10, background: 'var(--theme-surface-soft)', color: 'var(--text-muted)' }}>
      <span>BROWSER TEST ONLY · Simulated provider · No model calls</span>
      <Button size="sm" onClick={() => { location.hash = '#chat' }}>Normal Chat</Button>
      <Button size="sm" onClick={() => { location.hash = '#chat/study' }}>Study</Button>
      <Button size="sm" onClick={() => applyThemeSettings({ theme: document.documentElement.classList.contains('dark') ? 'light' : 'dark', themeColor: 'neutral', customThemes: [] })}>Toggle theme</Button>
    </div>
    <div style={{ display: 'flex', flex: 1, minHeight: 0 }}><Chat onSettingsChange={() => {}} /></div>
  </div>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
