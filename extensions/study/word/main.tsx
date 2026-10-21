import React, { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { Button } from '../../../src/components/Button'
import { Input, Select } from '../../../src/settings/public/controls'
import { AppNoticeHost, ErrorNotice, OperationNotice } from '../../../src/components/AppNotice'
import { applyThemeSettings } from '../../../src/theme/theme'
import { styles, type Reference, type Style } from './citations'
import { documentStyle, updateDocument } from './document'
import '../../../src/styles/app.css'
import './word.css'

export function App({ inWord }: { inWord: boolean }) {
  const [code, setCode] = useState(() => sessionStorage.getItem('study-connection') ?? '')
  const [items, setItems] = useState<Reference[]>([])
  const [connected, setConnected] = useState(false)
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<string[]>([])
  const [locator, setLocator] = useState('')
  const [style, setStyle] = useState<Style>(() => inWord ? documentStyle() : 'gbt')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  async function run(action: () => Promise<void>) {
    if (busy) return
    setBusy(true); setError(''); setNotice('')
    try { await action() } catch (e) { setError(e instanceof Error ? e.message : String(e)) } finally { setBusy(false) }
  }
  async function load() {
    const response = await fetch('/library', { headers: { Authorization: `Bearer ${code.trim()}` }, signal: AbortSignal.timeout(10000) })
    if (!response.ok) { setConnected(false); throw new Error(response.status === 401 ? '连接码无效或已撤销，请在 Study 重新复制连接码。' : '文献库读取失败，请检查 Kivio 是否正在运行。') }
    const data: unknown = await response.json()
    if (!Array.isArray(data) || data.some(r => !r || typeof r.id !== 'string' || typeof r.name !== 'string' || !Array.isArray(r.bibliography?.authors))) throw new Error('文献库响应无效，请更新 Kivio 后重试。')
    setItems(data); setConnected(true); sessionStorage.setItem('study-connection', code.trim())
    return data as Reference[]
  }
  async function act(action: 'insert' | 'edit' | 'refresh' | 'bibliography') {
    await run(async () => {
      const latest = await load()
      await updateDocument(latest, style, action, selected.map(id => ({ id, ...(locator.trim() ? { locator: locator.trim(), label: 'page' as const } : {}) })))
      setNotice(action === 'insert' ? '引用已插入，编号和参考文献已更新。' : '文档引用与参考文献已更新。')
    })
  }
  const terms = query.toLowerCase().trim().split(/\s+/)
  const filtered = items.filter(r => terms.every(t => `${r.name} ${r.bibliography.authors.join(' ')} ${r.bibliography.year} ${r.bibliography.doi}`.toLowerCase().includes(t)))
  return <main className="word-pane custom-scrollbar">
    <AppNoticeHost /><ErrorNotice message={error} /><OperationNotice message={notice} duration={5000} />
    <header><h1>Kivio Study</h1><p>从阅读到写作，引用本机文献。</p></header>
    {!inWord && <p role="status">此页面需在 Word 的 Kivio Study 加载项中打开，才能编辑文档。这里可检查文献连接。</p>}
    <section aria-label="连接 Study"><label>连接码<Input aria-label="连接码" type="password" value={code} onChange={setCode} placeholder="从 Study → 写作引用复制" /></label>
      <div className="word-actions"><Button disabled={busy || !code.trim()} onClick={() => void run(async () => { await load(); setNotice('已连接本机 Study 文献库。') })}>{connected ? '重新连接' : '连接 Study'}</Button>
      {connected && <Button variant="ghost" disabled={busy} onClick={() => { sessionStorage.removeItem('study-connection'); setCode(''); setConnected(false); setItems([]); setSelected([]) }}>断开</Button>}</div>
    </section>
    <section aria-label="文档引用"><label>引用格式<Select ariaLabel="引用格式" value={style} onChange={s => setStyle(s as Style)} options={Object.entries(styles).map(([value, s]) => ({ value, label: s.label }))} /></label>
      <div className="word-actions"><Button disabled={busy || !connected || !inWord} onClick={() => void act('refresh')}>刷新全文引用</Button><Button disabled={busy || !connected || !inWord} onClick={() => void act('bibliography')}>插入参考文献</Button></div>
      <p>修改格式、增删或移动引用后，点击刷新。参考文献随引用一起更新。</p>
    </section>
    <section aria-label="选择文献"><Input aria-label="搜索文献" placeholder="搜索标题、作者、年份或 DOI" value={query} onChange={setQuery} />
      <p>{connected ? `${filtered.length} 篇文献 · 已选 ${selected.length} 篇` : '连接后显示 Study 中的文献。'}</p>
      <div className="word-references custom-scrollbar">{filtered.map(r => <label key={r.id} className="word-reference"><input type="checkbox" checked={selected.includes(r.id)} disabled={busy} onChange={e => setSelected(ids => e.target.checked ? [...ids, r.id] : ids.filter(id => id !== r.id))} /><span><strong>{r.name}</strong><small>{r.bibliography.authors.join('; ')} · {r.bibliography.year || '年份待补充'}</small></span></label>)}</div>
      {connected && !filtered.length && <p>没有匹配文献。请在 Study 添加文献或修改搜索条件。</p>}
      <label>引用页码（可选）<Input aria-label="引用页码" value={locator} onChange={setLocator} placeholder="例如 12–15，应用于所选文献" /></label>
      <div className="word-actions"><Button variant="primary" disabled={busy || !inWord || !connected || !selected.length} onClick={() => void act('insert')}>插入引用</Button><Button disabled={busy || !inWord || !connected || !selected.length} onClick={() => void act('edit')}>替换当前引用</Button><Button variant="ghost" disabled={busy || !selected.length} onClick={() => setSelected([])}>清除选择</Button></div>
    </section>
    {busy && <p role="status">正在处理，请稍候…</p>}
    <footer>引用排版：<a href="https://citeproc-js.readthedocs.io/" target="_blank" rel="noreferrer">citeproc-js</a> · <a href="https://citationstyles.org/" target="_blank" rel="noreferrer">Citation Style Language</a><br />citeproc-js © Frank Bennett · AGPL-3.0-or-later<br /><a href="https://github.com/ZMGID/kivio" target="_blank" rel="noreferrer">插件源代码</a></footer>
  </main>
}
applyThemeSettings({ theme: 'system', themeColor: 'default', customThemes: [] })
const mount = (inWord: boolean) => createRoot(document.getElementById('root')!).render(<React.StrictMode><App inWord={inWord} /></React.StrictMode>)
if (typeof Office !== 'undefined') {
  void Office.onReady(info => mount(info.host === Office.HostType.Word && Office.context.requirements.isSetSupported('WordApi', '1.3')))
} else mount(false)
