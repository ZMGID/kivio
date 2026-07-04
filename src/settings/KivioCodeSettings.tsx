import { useEffect, useRef, useState } from 'react'
import { api, type KivioCodeConfig, type ModelProvider } from '../api/tauri'
import { ModelPairSelect } from './ModelPairSelect'
import { Select, SettingsGroup, SettingRow, TextArea, Toggle } from './components'

type Lang = 'zh' | 'zh-TW' | 'en'

interface KivioCodeSettingsProps {
  lang: Lang
  providers: ModelProvider[]
}

const DEFAULT_CONFIG: KivioCodeConfig = {
  readClaudeDir: true,
  defaultProviderId: '',
  defaultModel: '',
  approvalPolicy: 'auto',
}

/**
 * "Kivio Code" 設定頁:讀寫 kivio-code CLI 的獨立配置(<app_data>/kivio-code/config.json)。
 * 與共享 Settings 分開儲存,故走專用命令 get/saveKivioCodeConfig(參考 UsageStatsPanel 的取數方式)。
 * 改動即時落盤。模型選擇器複用 ModelPairSelect;空選項表示"跟隨 Chat 模型"。
 */
export function KivioCodeSettings({ lang, providers }: KivioCodeSettingsProps) {
  const zh = lang === 'zh' || lang === 'zh-TW'
  const [config, setConfig] = useState<KivioCodeConfig | null>(null)
  const [instructions, setInstructions] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  // 「安裝命令列工具」按鈕的狀態:installing 期間停用,result 顯示成功/已裝/失敗提示。
  const [installing, setInstalling] = useState(false)
  const [installMessage, setInstallMessage] = useState<{ ok: boolean; text: string } | null>(null)
  // 全域指令的防抖落盤:輸入停止 ~700ms 後寫一次,避免每次按鍵都寫磁碟。
  const instrTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .getKivioCodeConfig()
      .then((cfg) => {
        if (!cancelled) setConfig({ ...DEFAULT_CONFIG, ...cfg })
      })
      .catch((err) => {
        if (!cancelled) {
          console.error('Failed to load kivio-code config:', err)
          setConfig({ ...DEFAULT_CONFIG })
          setError(String(err))
        }
      })
    api
      .getKivioCodeGlobalInstructions()
      .then((text) => {
        if (!cancelled) setInstructions(text)
      })
      .catch((err) => {
        if (!cancelled) {
          console.error('Failed to load kivio-code global instructions:', err)
          setInstructions('')
        }
      })
    return () => {
      cancelled = true
      if (instrTimer.current) clearTimeout(instrTimer.current)
    }
  }, [])

  if (!config) {
    return (
      <SettingsGroup>
        <div className="px-1 py-4 text-[13px] text-neutral-400">
          {zh ? '載入中…' : 'Loading…'}
        </div>
      </SettingsGroup>
    )
  }

  const update = (patch: Partial<KivioCodeConfig>) => {
    const next = { ...config, ...patch }
    setConfig(next)
    api.saveKivioCodeConfig(next).catch((err) => {
      console.error('Failed to save kivio-code config:', err)
      setError(String(err))
    })
  }

  const updateInstructions = (value: string) => {
    setInstructions(value)
    if (instrTimer.current) clearTimeout(instrTimer.current)
    instrTimer.current = setTimeout(() => {
      api.saveKivioCodeGlobalInstructions(value).catch((err) => {
        console.error('Failed to save kivio-code global instructions:', err)
        setError(String(err))
      })
    }, 700)
  }

  const installCli = () => {
    setInstalling(true)
    setInstallMessage(null)
    api
      .installCliCommand()
      .then((result) => {
        setInstallMessage({ ok: result.ok, text: result.message })
      })
      .catch((err) => {
        console.error('Failed to install kivio CLI command:', err)
        setInstallMessage({ ok: false, text: String(err) })
      })
      .finally(() => setInstalling(false))
  }

  return (
    <>
      <SettingsGroup title={zh ? '預設模型與許可權' : 'Default model & permissions'}>
        <SettingRow
          label={zh ? '預設模型' : 'Default model'}
          description={
            zh
              ? '留空則跟隨 Chat 預設模型。命令列 --model / --provider 仍優先生效。'
              : 'Leave unset to follow the Chat default model. CLI --model / --provider still take precedence.'
          }
        >
          <ModelPairSelect
            providerId={config.defaultProviderId || ''}
            model={config.defaultModel || ''}
            providers={providers}
            onChange={(providerId, model) =>
              update({ defaultProviderId: providerId, defaultModel: model })
            }
            inheritLabel={zh ? '跟隨 Chat 模型' : 'Follow Chat model'}
          />
        </SettingRow>

        <SettingRow
          label={zh ? '工具審批策略' : 'Tool approval policy'}
          description={
            zh
              ? '決定 kivio-code 執行工具前是否需要確認。命令列 --no-approve 會強制為"每次確認"。'
              : 'Whether kivio-code confirms before running tools. CLI --no-approve forces "confirm every call".'
          }
        >
          <Select
            value={config.approvalPolicy || 'auto'}
            onChange={(approvalPolicy) => update({ approvalPolicy })}
            options={[
              { value: 'auto', label: zh ? '完全訪問' : 'Full access' },
              {
                value: 'readonly_auto_sensitive_confirm',
                label: zh ? '敏感確認' : 'Sensitive confirmation',
              },
              { value: 'always_confirm', label: zh ? '每次確認' : 'Confirm every call' },
            ]}
          />
        </SettingRow>

        <SettingRow
          label={zh ? '讀取 CLAUDE.md / .claude 上下文' : 'Read CLAUDE.md / .claude context'}
          description={
            zh
              ? '開啟後 kivio-code 會讀取專案與全域的 CLAUDE.md / .claude 指令檔案(跨工具相容)。'
              : "When on, kivio-code reads project and global CLAUDE.md / .claude instruction files for cross-tool compatibility."
          }
        >
          <Toggle
            checked={config.readClaudeDir}
            onChange={(readClaudeDir) => update({ readClaudeDir })}
          />
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title={zh ? '全域指令' : 'Global instructions'}>
        <div className="px-1 pb-2 text-[12px] leading-relaxed text-neutral-500 dark:text-neutral-400">
          {zh
            ? '每次執行 kivio-code 都會注入的全域指令。專案根目錄的 KIVIO.md / AGENTS.md 會在其後疊加。'
            : "Global instructions injected on every kivio-code run. A project's root KIVIO.md / AGENTS.md layers on top."}
        </div>
        <TextArea
          value={instructions ?? ''}
          onChange={updateInstructions}
          rows={10}
          mono
          placeholder={
            zh
              ? '# 全域指令\n\n例如:始終用中文回覆;提交資訊遵循 Conventional Commits…'
              : '# Global instructions\n\ne.g. Always answer in English; follow Conventional Commits for messages…'
          }
        />
      </SettingsGroup>

      <SettingsGroup title={zh ? '啟動方式' : 'How to launch'}>
        <div className="px-1 py-2 text-[13px] leading-relaxed text-neutral-500 dark:text-neutral-400">
          {zh ? (
            <>
              安裝 CLI 後在專案目錄執行{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                kivio code
              </code>
              ；讀取上方設定，{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                --model provider:model
              </code>{' '}
              可臨時覆蓋模型。
            </>
          ) : (
            <>
              Install the CLI, then run{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                kivio code
              </code>{' '}
              in a project; uses settings above. Pass{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                --model provider:model
              </code>{' '}
              to override for one run.
            </>
          )}
        </div>

        <SettingRow
          label={zh ? '安裝命令列工具' : 'Install command line tool'}
          description={
            zh
              ? '把 kivio 命令註冊進使用者 PATH(Windows)/軟鏈到 ~/.local/bin(macOS),裝好後在新終端可直接用 kivio code。'
              : 'Register the kivio command on your user PATH (Windows) / symlink it to ~/.local/bin (macOS). After installing, use kivio code in a new terminal.'
          }
        >
          <button
            type="button"
            onClick={installCli}
            disabled={installing}
            className="rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-[13px] font-medium text-neutral-700 transition hover:bg-neutral-50 disabled:cursor-not-allowed disabled:opacity-60 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700"
          >
            {installing
              ? zh
                ? '安裝中…'
                : 'Installing…'
              : zh
                ? '安裝'
                : 'Install'}
          </button>
        </SettingRow>

        {installMessage && (
          <div
            className={
              'px-1 pb-2 text-[12px] leading-relaxed ' +
              (installMessage.ok
                ? 'text-emerald-600 dark:text-emerald-400'
                : 'text-red-500 dark:text-red-400')
            }
          >
            {installMessage.text}
          </div>
        )}
      </SettingsGroup>

      {error && (
        <div className="px-1 py-2 text-[12px] text-red-500 dark:text-red-400">
          {(zh ? '儲存失敗:' : 'Save failed: ') + error}
        </div>
      )}
    </>
  )
}
