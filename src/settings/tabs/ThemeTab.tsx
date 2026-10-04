import { useEffect, useId, useRef, useState, type CSSProperties } from 'react'
import { Check, Copy, Download, Upload } from 'lucide-react'
import { Button } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { Input, Select, TextArea, Toggle } from '../public/controls'
import { SettingRow, SettingsGroup } from '../components'
import type { Settings } from '../../api/tauri'
import type { Lang } from '../../components/i18n'
import { copyToClipboard } from '../../utils/clipboard'
import { BUILTIN_THEMES, exportThemeJson, paletteVariables, parseThemeJson, resolveTheme } from '../../theme/theme'
import { THEME_FIELDS, type ThemeDefinition, type ThemePalette } from '../../theme/types'

const FIELD_LABELS: Record<keyof ThemePalette, string> = {
  surface: '主表面', surfaceSoft: '次级表面', surfaceMuted: '弱化表面',
  surfaceHover: '悬停表面', surfaceActive: '选中表面', surfaceTitlebar: '标题栏',
  border: '边框', borderStrong: '强调边框', text: '主文字', textMuted: '次级文字',
  textFaint: '弱化文字', accent: '强调色', accentHover: '强调色悬停',
  accentSoft: '强调色背景', onAccent: '强调色上的文字', danger: '危险色', dangerSoft: '危险色背景',
}

const COLOR_GROUPS: { zh: string; en: string; fields: (keyof ThemePalette)[] }[] = [
  { zh: '表面与层级', en: 'Surfaces', fields: ['surface', 'surfaceSoft', 'surfaceMuted', 'surfaceHover', 'surfaceActive', 'surfaceTitlebar'] },
  { zh: '文字与边框', en: 'Text & borders', fields: ['text', 'textMuted', 'textFaint', 'border', 'borderStrong'] },
  { zh: '强调色', en: 'Accent', fields: ['accent', 'accentHover', 'accentSoft', 'onAccent'] },
  { zh: '危险状态', en: 'Danger', fields: ['danger', 'dangerSoft'] },
]

const BUILTIN_LABELS: Record<string, string> = {
  neutral: '中性', warm: '暖白', cool: '冷白',
  graphite: '石墨', blossom: '樱粉', grove: '森林',
  ocean: '海湾', ember: '余烬', iris: '鸢尾',
  white: '纯白', nord: '北欧 Nord', solarized: '日晒 Solarized',
}

function ThemePreview({ theme, mode, lang }: { theme: ThemeDefinition; mode: 'light' | 'dark'; lang: Lang }) {
  // An incomplete hex value is an editor state, never an applied CSS value.
  const palette = Object.fromEntries(THEME_FIELDS.map(field => [
    field, /^#[0-9a-f]{6}$/i.test(theme[mode][field]) ? theme[mode][field] : BUILTIN_THEMES[0][mode][field],
  ])) as ThemePalette
  return (
    <div aria-label={lang === 'zh' ? `${mode === 'light' ? '浅色' : '深色'}预览` : `${mode} preview`}
      style={paletteVariables(palette) as CSSProperties}
      className="theme-preview grid grid-cols-[72px_minmax(0,1fr)] overflow-hidden rounded-xl border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] text-[var(--text)]">
      <aside className="flex flex-col gap-3 bg-[var(--theme-surface-soft)] p-3 text-xs">
        <strong>Kivio</strong>
        <span className="rounded-md bg-[var(--theme-surface-active)] p-2">{lang === 'zh' ? '对话' : 'Chat'}</span>
        <span className="text-[var(--text-muted)]">{lang === 'zh' ? '项目' : 'Projects'}</span>
      </aside>
      <div className="flex min-w-0 flex-col gap-3 p-3 text-xs">
        <strong>{lang === 'zh' ? '你的 AI 工作空间' : 'Your AI workspace'}</strong>
        <div className="max-w-full self-end rounded-lg bg-[var(--theme-surface-muted)] p-2">{lang === 'zh' ? '帮我整理一下思路' : 'Help organize my ideas'}</div>
        <p className="text-[var(--text-muted)]">{lang === 'zh' ? '从一个清晰的问题开始。' : 'Start with a clear question.'}</p>
        <div className="flex items-center justify-between rounded-lg border border-[var(--theme-surface-border-strong)] p-2">
          <span className="text-[var(--text-faint)]">{lang === 'zh' ? '输入消息…' : 'Message…'}</span>
          <span className="rounded-md bg-[var(--accent)] px-3 py-1 text-[var(--text-onaccent)]">↑</span>
        </div>
        <div className="flex gap-3"><span className="text-[var(--accent)]">{lang === 'zh' ? '链接' : 'Link'}</span><span className="text-[var(--danger)]">{lang === 'zh' ? '错误提示' : 'Error'}</span></div>
      </div>
    </div>
  )
}

export function ThemeTab({ settings, lang, draft, onDraftChange, onCommit }: {
  settings: Settings
  lang: Lang
  draft: ThemeDefinition | null
  onDraftChange: (draft: ThemeDefinition | null) => void
  onCommit: (update: (current: Settings) => Settings) => Promise<void>
}) {
  const zh = lang === 'zh'
  const [selectedId, setSelectedId] = useState(settings.themeColor)
  const [paletteMode, setPaletteMode] = useState<'light' | 'dark'>('light')
  const [jsonMode, setJsonMode] = useState<'import' | 'export' | null>(null)
  const [json, setJson] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState<'save' | 'apply' | 'settings' | 'delete' | 'copy' | null>(null)
  const [notice, setNotice] = useState('')
  const flight = useRef(false)
  const cardRefs = useRef<(HTMLButtonElement | null)[]>([])
  const editorRef = useRef<HTMLElement | null>(null)
  const dialogRef = useRef<HTMLDialogElement | null>(null)
  const inputId = useId()
  const editing = Boolean(draft)
  const themes = [...BUILTIN_THEMES, ...(settings.customThemes ?? [])]
  const selected = themes.find(theme => theme.id === selectedId) ?? resolveTheme(settings)
  const preview = draft ?? selected
  const isBuiltin = BUILTIN_THEMES.some(theme => theme.id === preview.id)
  const previewName = zh && isBuiltin ? BUILTIN_LABELS[preview.id] : preview.name
  const active = settings.themeColor === selected.id
  const nameValid = draft && draft.name.trim().length > 0 && draft.name.trim().length <= 80 && !/\p{Cc}/u.test(draft.name.trim())
  const invalidFields = draft ? THEME_FIELDS.filter(field => !/^#[0-9a-f]{6}$/i.test(draft[paletteMode][field])) : []
  const validDraft = nameValid && draft && THEME_FIELDS.every(field => /^#[0-9a-f]{6}$/i.test(draft.light[field]) && /^#[0-9a-f]{6}$/i.test(draft.dark[field]))

  useEffect(() => {
    if (editing) editorRef.current?.scrollIntoView?.({ block: 'start', behavior: 'instant' })
  }, [editing])

  useEffect(() => {
    if (!jsonMode) return
    const dialog = dialogRef.current
    if (!dialog) return
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    dialog.showModal()
    dialog.querySelector('textarea')?.focus()
    return () => {
      dialog.close()
      if (previousFocus?.isConnected) previousFocus.focus()
    }
  }, [jsonMode])

  const run = async (kind: NonNullable<typeof busy>, action: () => Promise<void>) => {
    if (flight.current) return
    flight.current = true
    setBusy(kind)
    setError('')
    setNotice('')
    try { await action() }
    catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)) }
    finally { flight.current = false; setBusy(null) }
  }
  const selectTheme = (id: string) => {
    setSelectedId(id)
    setError('')
    setNotice('')
  }
  const copy = () => {
    onDraftChange({ ...preview, id: crypto.randomUUID(), name: `${previewName} ${zh ? '副本' : 'copy'}`, light: { ...preview.light }, dark: { ...preview.dark } })
    setPaletteMode('light')
    setError('')
    setNotice('')
  }
  const saveDraft = () => void run('save', async () => {
    if (!draft || !validDraft) return
    const candidate = { ...draft, name: draft.name.trim() }
    await onCommit(current => ({ ...current, customThemes: [...(current.customThemes ?? []).filter(theme => theme.id !== candidate.id), candidate] }))
    setSelectedId(candidate.id)
    onDraftChange(null)
    setNotice(settings.themeColor === candidate.id
      ? (zh ? '主题已保存，当前界面已更新。' : 'Theme saved and applied.')
      : (zh ? '主题已保存；点击「应用主题」切换。' : 'Theme saved. Select Apply theme to switch.'))
  })
  const remove = () => void run('delete', async () => {
    if (isBuiltin || draft) return
    if (!await confirmDialog({ message: zh ? `删除主题「${selected.name}」？${active ? '当前界面将恢复为中性主题。' : ''}` : `Delete theme “${selected.name}”?${active ? ' Neutral will be restored.' : ''}`, danger: true })) return
    await onCommit(current => ({ ...current,
      customThemes: (current.customThemes ?? []).filter(theme => theme.id !== selected.id),
      themeColor: current.themeColor === selected.id ? 'neutral' : current.themeColor,
    }))
    setSelectedId(active ? 'neutral' : settings.themeColor)
    setNotice(zh ? '主题已删除。' : 'Theme deleted.')
  })

  return (
    <div className="theme-page" aria-label={zh ? '主题管理' : 'Theme manager'} aria-busy={Boolean(busy)}>
      {!draft && <SettingsGroup title={zh ? '显示模式与材质' : 'Mode & material'}>
        <SettingRow label={zh ? '明暗模式' : 'Appearance'}>
          <Select value={settings.theme} disabled={Boolean(busy)} className="w-40" ariaLabel={zh ? '明暗模式' : 'Appearance mode'}
            options={[{ value: 'system', label: zh ? '跟随系统' : 'System' }, { value: 'light', label: zh ? '浅色' : 'Light' }, { value: 'dark', label: zh ? '深色' : 'Dark' }]}
            onChange={value => void run('settings', () => onCommit(current => ({ ...current, theme: value as Settings['theme'] })))} />
        </SettingRow>
        <SettingRow label={zh ? '半透明侧边栏' : 'Translucent sidebar'}>
          <Toggle checked={settings.translucentSidebar} disabled={Boolean(busy)} ariaLabel={zh ? '半透明侧边栏' : 'Translucent sidebar'}
            onChange={value => void run('settings', () => onCommit(current => ({ ...current, translucentSidebar: value })))} />
        </SettingRow>
      </SettingsGroup>}

      {!draft && <section className="theme-library" aria-labelledby={`${inputId}-library`}>
        <div className="theme-section-heading">
          <div><h3 id={`${inputId}-library`}>{zh ? '主题库' : 'Theme library'}</h3><p>{zh ? '选择查看预览，再应用到界面。' : 'Select to preview, then apply to your interface.'}</p></div>
          <Button size="sm" disabled={Boolean(busy)} onClick={() => { setJsonMode('import'); setJson(''); setError(''); setNotice('') }}><Upload size={13} aria-hidden />{zh ? '导入 JSON' : 'Import JSON'}</Button>
        </div>
        <div className="theme-gallery" role="radiogroup" aria-label={zh ? '主题库' : 'Theme library'}>
          {themes.map((theme, index) => {
            const chosen = selected.id === theme.id
            const current = settings.themeColor === theme.id
            const builtin = BUILTIN_THEMES.some(item => item.id === theme.id)
            const name = zh && builtin ? BUILTIN_LABELS[theme.id] : theme.name
            return (
              <button key={theme.id} type="button" role="radio" aria-checked={chosen} tabIndex={chosen ? 0 : -1}
                aria-label={`${name} · ${current ? (zh ? '正在使用' : 'Active') : builtin ? (zh ? '内置' : 'Built-in') : (zh ? '自定义' : 'Custom')}`}
                ref={node => { cardRefs.current[index] = node }} disabled={Boolean(busy)}
                className={`theme-card${chosen ? ' is-selected' : ''}`} onClick={() => selectTheme(theme.id)}
                onKeyDown={event => {
                  const step = ['ArrowRight', 'ArrowDown'].includes(event.key) ? 1 : ['ArrowLeft', 'ArrowUp'].includes(event.key) ? -1 : 0
                  if (!step && event.key !== 'Home' && event.key !== 'End') return
                  event.preventDefault()
                  const next = event.key === 'Home' ? 0 : event.key === 'End' ? themes.length - 1 : (index + step + themes.length) % themes.length
                  selectTheme(themes[next].id)
                  cardRefs.current[next]?.focus()
                }}>
                <span className="theme-card-pair" aria-hidden="true">
                  {(['light', 'dark'] as const).map(mode => (
                    <span key={mode} className="theme-card-mini" style={{ background: theme[mode].surface, borderColor: theme[mode].border }}>
                      <span className="theme-card-mini-sidebar" style={{ background: theme[mode].surfaceSoft }} />
                      <span className="theme-card-mini-content">
                        <span className="theme-card-mini-line" style={{ background: theme[mode].text }} />
                        <span className="theme-card-mini-line short" style={{ background: theme[mode].textMuted }} />
                        <span className="theme-card-mini-message" style={{ background: theme[mode].surfaceMuted }} />
                        <span className="theme-card-mini-accent" style={{ background: theme[mode].accent }} />
                      </span>
                    </span>
                  ))}
                </span>
                <span className="theme-card-name"><span title={name}>{name}</span>{chosen && <Check size={14} aria-hidden />}</span>
                <span className="theme-card-meta">{current ? (zh ? '正在使用' : 'Active') : builtin ? (zh ? '内置主题' : 'Built-in') : (zh ? '自定义主题' : 'Custom')}</span>
              </button>
            )
          })}
        </div>
      </section>}

      <section className="theme-workspace" ref={editorRef} aria-labelledby={`${inputId}-workspace`}>
        <header className="theme-workspace-header">
          <div className="min-w-0"><h3 id={`${inputId}-workspace`} className="truncate" title={previewName}>{previewName}</h3>
            <p>{draft ? (zh ? '编辑草稿 · 预览不会改变当前界面' : 'Unsaved draft · preview only') : active ? (zh ? '当前主题 · 已应用' : 'Current theme · applied') : (zh ? '仅预览 · 尚未应用' : 'Preview only · not applied')}</p>
          </div>
          <div className="theme-workspace-actions">
            {draft ? <>
              <Button disabled={Boolean(busy)} onClick={() => { onDraftChange(null); setError(''); setNotice('') }}>{zh ? '取消编辑' : 'Cancel editing'}</Button>
              <Button variant="primary" disabled={Boolean(busy) || !validDraft} onClick={saveDraft}>{busy === 'save' ? (zh ? '保存中…' : 'Saving…') : (zh ? '保存主题' : 'Save theme')}</Button>
            </> : <>
              <Button size="sm" disabled={Boolean(busy)} onClick={copy}><Copy size={13} aria-hidden />{zh ? '复制为自定义主题' : 'Duplicate theme'}</Button>
              {!isBuiltin && <Button size="sm" disabled={Boolean(busy)} onClick={() => { onDraftChange({ ...selected, light: { ...selected.light }, dark: { ...selected.dark } }); setError(''); setNotice('') }}>{zh ? '编辑主题' : 'Edit theme'}</Button>}
              <Button size="sm" variant="primary" disabled={Boolean(busy) || active}
                onClick={() => void run('apply', async () => { await onCommit(current => ({ ...current, themeColor: selected.id })); setNotice(zh ? '主题已应用。' : 'Theme applied.') })}>{busy === 'apply' ? (zh ? '应用中…' : 'Applying…') : active ? (zh ? '已应用' : 'Applied') : (zh ? '应用主题' : 'Apply theme')}</Button>
            </>}
          </div>
        </header>
        {!jsonMode && error && <p role="alert" className="theme-feedback error">{error}</p>}
        {!jsonMode && notice && <p role="status" className="theme-feedback">{notice}</p>}

        <div className={`theme-workspace-body${draft ? ' is-editing' : ''}`}>
          {draft ? <>
            <div className="theme-editor">
              <label htmlFor={`${inputId}-name`} className="theme-field-name">{zh ? '主题名称' : 'Theme name'}</label>
              <Input id={`${inputId}-name`} aria-label={zh ? '主题名称' : 'Theme name'} autoFocus value={draft.name} maxLength={80} disabled={Boolean(busy)}
                aria-invalid={!nameValid} aria-describedby={!nameValid ? `${inputId}-name-error` : undefined} onChange={name => onDraftChange({ ...draft, name })} />
              {!nameValid && <p id={`${inputId}-name-error`} className="theme-field-error">{zh ? '请输入 1–80 个字符的名称，不含控制字符。' : 'Use 1–80 characters without control characters.'}</p>}
              <div className="theme-palette-heading"><h4>{zh ? '编辑色板' : 'Edit palette'}</h4>
                <div className="kv-seg" role="group" aria-label={zh ? '编辑色板' : 'Palette to edit'}>
                  {(['light', 'dark'] as const).map(mode => <button key={mode} type="button" className={paletteMode === mode ? 'active' : ''} aria-pressed={paletteMode === mode} disabled={Boolean(busy)}
                    onClick={() => setPaletteMode(mode)}>{mode === 'light' ? (zh ? '浅色' : 'Light') : (zh ? '深色' : 'Dark')}{THEME_FIELDS.some(field => !/^#[0-9a-f]{6}$/i.test(draft[mode][field])) ? ' !' : ''}</button>)}
                </div>
              </div>
              {COLOR_GROUPS.map(group => <fieldset key={group.en} className="theme-color-group" disabled={Boolean(busy)}>
                <legend>{zh ? group.zh : group.en}</legend>
                <div className="theme-color-grid">{group.fields.map(field => {
                  const color = draft[paletteMode][field]
                  const invalid = !/^#[0-9a-f]{6}$/i.test(color)
                  return <div className="theme-color-field" key={field}>
                    <label htmlFor={`${inputId}-${paletteMode}-${field}`}>{zh ? FIELD_LABELS[field] : field}</label>
                    <div className="theme-color-inputs">
                      <span className="theme-color-picker"><Input type="color" aria-label={`${paletteMode}.${field}.picker`} title={zh ? `选择${FIELD_LABELS[field]}` : `Pick ${field}`} value={invalid ? BUILTIN_THEMES[0][paletteMode][field] : color}
                        onChange={next => onDraftChange({ ...draft, [paletteMode]: { ...draft[paletteMode], [field]: next } })} /></span>
                      <Input id={`${inputId}-${paletteMode}-${field}`} mono className="min-w-0" aria-label={`${paletteMode}.${field}`} value={color} maxLength={7} spellCheck={false} aria-invalid={invalid}
                        aria-describedby={invalid ? `${inputId}-${field}-error` : undefined}
                        onChange={next => onDraftChange({ ...draft, [paletteMode]: { ...draft[paletteMode], [field]: next } })} />
                    </div>
                    {invalid && <span id={`${inputId}-${field}-error`} className="theme-field-error">{zh ? '使用 #RRGGBB 格式' : 'Use #RRGGBB'}</span>}
                  </div>
                })}</div>
              </fieldset>)}
              {!validDraft && <p role="alert" className="theme-field-error">{!nameValid ? (zh ? '请先填写有效名称。' : 'Enter a valid name first.') : invalidFields.length ? (zh ? '请修正当前色板中标出的颜色。' : 'Correct the marked colors in this palette.') : (zh ? '另一套色板仍有无效颜色，请切换检查。' : 'The other palette still has invalid colors. Switch to check it.')}</p>}
            </div>
            <aside className="theme-live-preview"><h4>{zh ? '实时预览' : 'Live preview'} · {paletteMode === 'light' ? (zh ? '浅色' : 'Light') : (zh ? '深色' : 'Dark')}</h4><ThemePreview theme={preview} mode={paletteMode} lang={lang} />
              <p>{zh ? '取色器与 HEX 输入同步；未完成的颜色暂用默认值预览，不会保存。' : 'The picker and HEX input stay in sync. Incomplete colors preview a default and cannot be saved.'}</p></aside>
          </> : <div className="theme-preview-pair">
            {(['light', 'dark'] as const).map(mode => <div key={mode}><h4>{mode === 'light' ? (zh ? '浅色预览' : 'Light preview') : (zh ? '深色预览' : 'Dark preview')}</h4><ThemePreview theme={preview} mode={mode} lang={lang} /></div>)}
          </div>}
        </div>
        {!draft && <footer className="theme-share-actions">
          <Button size="sm" variant="ghost" disabled={Boolean(busy)} onClick={() => { setJsonMode('export'); setJson(exportThemeJson(selected)); setError(''); setNotice('') }}><Download size={13} aria-hidden />{zh ? '导出 JSON' : 'Export JSON'}</Button>
          {!isBuiltin && <Button size="sm" variant="danger" disabled={Boolean(busy)} onClick={remove}>{busy === 'delete' ? (zh ? '删除中…' : 'Deleting…') : (zh ? '删除主题' : 'Delete theme')}</Button>}
        </footer>}
      </section>

      {jsonMode && <dialog ref={dialogRef} className="kv-modal theme-json-dialog" aria-labelledby={`${inputId}-json-title`} aria-describedby={`${inputId}-json-help`}
        onKeyDown={event => event.stopPropagation()} onCancel={event => { event.preventDefault(); if (!busy) setJsonMode(null) }}>
        <h3 id={`${inputId}-json-title`}>{jsonMode === 'import' ? (zh ? '导入主题 JSON' : 'Import theme JSON') : (zh ? '导出主题 JSON' : 'Export theme JSON')}</h3>
        <p id={`${inputId}-json-help`}>{jsonMode === 'import' ? (zh ? '粘贴完整主题 JSON。读取后进入草稿预览，不会覆盖已有主题。' : 'Paste a complete theme document. Import opens an independent draft without replacing existing themes.') : (zh ? '复制下方 JSON，即可分享主题的浅色与深色色板。' : 'Copy this JSON to share both palettes.')}</p>
        <div className="theme-json-body custom-scrollbar"><TextArea aria-label={zh ? '主题 JSON' : 'Theme JSON'} value={json} onChange={setJson} readOnly={jsonMode === 'export'} rows={12} mono />
          {error && <p role="alert" className="theme-feedback error">{error}</p>}{notice && <p role="status" className="theme-feedback">{notice}</p>}
        </div>
        <div className="theme-json-actions"><Button disabled={Boolean(busy)} onClick={() => setJsonMode(null)}>{zh ? '关闭' : 'Close'}</Button>
          {jsonMode === 'import' ? <Button variant="primary" disabled={Boolean(busy) || !json.trim()} onClick={() => {
            try { const imported = parseThemeJson(json); setJsonMode(null); setPaletteMode('light'); onDraftChange(imported); setError(''); setNotice('') }
            catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)) }
          }}>{zh ? '读取并预览' : 'Read & preview'}</Button> : <Button variant="primary" disabled={Boolean(busy)} onClick={() => void run('copy', async () => {
            if (!await copyToClipboard(json)) throw new Error(zh ? '复制失败，请手动复制 JSON。' : 'Copy failed. Select and copy the JSON manually.')
            setNotice(zh ? 'JSON 已复制。' : 'JSON copied.')
          })}>{zh ? '复制 JSON' : 'Copy JSON'}</Button>}
        </div>
      </dialog>}
    </div>
  )
}
