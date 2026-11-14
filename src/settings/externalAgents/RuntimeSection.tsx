import { useState } from 'react'
import { Eye, EyeOff, FolderOpen, FolderSearch, Plus, X } from 'lucide-react'
import { open } from '@tauri-apps/plugin-dialog'
import { externalCliSettingsApi } from '../../api/externalCliSettings'
import type { ExternalCliAgentConfig } from '../../api/tauri'
import { notifyError } from '../../components/appNoticeStore'
import { Button, IconButton } from '../../components/Button'
import { i18n, type Lang } from '../../components/i18n'
import { DraftInput, Row, Section } from './shared'

type EnvPair = NonNullable<ExternalCliAgentConfig['env']>[number]
type Strings = (typeof i18n)[Lang]

/** 值多半是密钥的变量名：默认遮住，免得录屏 / 截图时露出来。 */
const SECRET_NAME = /(KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL)/i
const VALID_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/

function EnvRow({
  pair,
  autoFocus,
  t,
  onChange,
  onRemove,
}: {
  pair: EnvPair
  autoFocus: boolean
  t: Strings
  onChange: (patch: Partial<EnvPair>) => void
  onRemove: () => void
}) {
  // `null` = 跟着变量名走；用户点过眼睛之后以他的选择为准。
  const [revealed, setRevealed] = useState<boolean | null>(null)
  const hidden = !(revealed ?? !SECRET_NAME.test(pair.key))
  const invalid = pair.key !== '' && !VALID_NAME.test(pair.key)
  return (
    <div className="kv-cli-env">
      <DraftInput
        value={pair.key}
        onCommit={(key) => onChange({ key: key.trim() })}
        placeholder={t.externalAgentsEnvKey}
        aria-label={t.externalAgentsEnvKey}
        aria-invalid={invalid || undefined}
        title={invalid ? t.externalAgentsEnvNameInvalid : undefined}
        autoFocus={autoFocus}
        spellCheck={false}
        mono
      />
      <div className="kv-cli-env-value">
        <DraftInput
          type={hidden ? 'password' : 'text'}
          value={pair.value}
          onCommit={(value) => onChange({ value })}
          placeholder={t.externalAgentsEnvValue}
          aria-label={t.externalAgentsEnvValue}
          autoComplete="off"
          spellCheck={false}
          mono
        />
        <IconButton
          size="xs"
          variant="ghost"
          label={hidden ? t.externalAgentsShowValue : t.externalAgentsHideValue}
          onClick={() => setRevealed(hidden)}
        >
          {hidden ? <Eye size={13} /> : <EyeOff size={13} />}
        </IconButton>
      </div>
      <IconButton size="sm" variant="ghost" label={t.externalAgentsRemove} onClick={onRemove}>
        <X size={14} />
      </IconButton>
    </div>
  )
}

/**
 * 「运行环境」：Kivio 拉起这个 CLI 时用哪个可执行文件、带哪些环境变量、它的配置放在哪。
 * 没装的 CLI 也显示这一区——指对路径本身就是「装上」的一种方式。
 */
export function RuntimeSection({
  lang,
  agentId,
  detectedPath,
  configDir,
  config,
  onPatch,
}: {
  lang: Lang
  agentId: string
  detectedPath: string | null
  configDir: string | null
  config: ExternalCliAgentConfig
  onPatch: (patch: Partial<ExternalCliAgentConfig>) => void
}) {
  const t = i18n[lang]
  const env = config.env ?? []
  const customPath = config.path ?? ''
  // 刚点「添加变量」出来的那一行要拿到焦点；已有的行不抢。
  const [focusIndex, setFocusIndex] = useState<number | null>(null)

  const pickBinary = async () => {
    const picked = await open({ multiple: false, directory: false })
    if (typeof picked === 'string') onPatch({ path: picked })
  }
  const openConfigDir = async () => {
    try {
      await externalCliSettingsApi.openConfigDir(agentId)
    } catch (err) {
      void notifyError(err instanceof Error ? err.message : String(err))
    }
  }
  const addVariable = () => {
    setFocusIndex(env.length)
    onPatch({ env: [...env, { key: '', value: '' }] })
  }

  return (
    <Section title={t.externalAgentsRuntimeSection}>
      <div className="kv-cli-card">
        <div className="kv-row-stack kv-cli-stack">
          <label className="kv-row-label" htmlFor={`cli-binary-${agentId}`}>
            {t.externalAgentsBinarySection}
            {customPath && <span className="kv-tag">{t.externalAgentsBinaryCustomTag}</span>}
          </label>
          <div className="kv-cli-field">
            <DraftInput
              id={`cli-binary-${agentId}`}
              value={customPath}
              onCommit={(path) => onPatch({ path: path.trim() })}
              placeholder={detectedPath || t.externalAgentsBinaryNotFound}
              title={customPath || detectedPath || undefined}
              spellCheck={false}
              mono
            />
            <IconButton size="sm" label={t.externalAgentsBrowse} onClick={() => void pickBinary()}>
              <FolderSearch size={13} />
            </IconButton>
          </div>
          <p className="kv-row-desc">{t.externalAgentsBinaryHint}</p>
        </div>
        {configDir && (
          <Row label={t.externalAgentsConfigDir} desc={<code className="kv-cli-mono">{configDir}</code>}>
            <Button size="sm" onClick={() => void openConfigDir()}>
              <FolderOpen size={12} />
              {t.externalAgentsOpenDir}
            </Button>
          </Row>
        )}
        <Row label={t.externalAgentsEnvSection} desc={t.externalAgentsEnvDesc}>
          <Button size="sm" onClick={addVariable}>
            <Plus size={12} />
            {t.externalAgentsEnvAdd}
          </Button>
        </Row>
        {env.length > 0 && (
          <div className="kv-cli-envs">
            {env.map((pair, index) => (
              <EnvRow
                key={index}
                pair={pair}
                autoFocus={index === focusIndex}
                t={t}
                onChange={(patch) =>
                  onPatch({ env: env.map((item, i) => (i === index ? { ...item, ...patch } : item)) })
                }
                onRemove={() => {
                  setFocusIndex(null)
                  onPatch({ env: env.filter((_, i) => i !== index) })
                }}
              />
            ))}
          </div>
        )}
      </div>
    </Section>
  )
}
