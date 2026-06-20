import { useEffect, useRef, useState } from 'react'
import { api, type KivioCodeConfig, type ModelProvider } from '../api/tauri'
import { ModelPairSelect } from './ModelPairSelect'
import { Select, SettingsGroup, SettingRow, TextArea, Toggle } from './components'

type Lang = 'zh' | 'en'

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
 * "Kivio Code" 设置页:读写 kivio-code CLI 的独立配置(<app_data>/kivio-code/config.json)。
 * 与共享 Settings 分开存储,故走专用命令 get/saveKivioCodeConfig(参考 UsageStatsPanel 的取数方式)。
 * 改动即时落盘。模型选择器复用 ModelPairSelect;空选项表示"跟随 Chat 模型"。
 */
export function KivioCodeSettings({ lang, providers }: KivioCodeSettingsProps) {
  const zh = lang === 'zh'
  const [config, setConfig] = useState<KivioCodeConfig | null>(null)
  const [instructions, setInstructions] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  // 「安装命令行工具」按钮的状态:installing 期间禁用,result 显示成功/已装/失败提示。
  const [installing, setInstalling] = useState(false)
  const [installMessage, setInstallMessage] = useState<{ ok: boolean; text: string } | null>(null)
  // 全局指令的防抖落盘:输入停止 ~700ms 后写一次,避免每次按键都写磁盘。
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
          {zh ? '加载中…' : 'Loading…'}
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
      <SettingsGroup title={zh ? '默认模型与权限' : 'Default model & permissions'}>
        <SettingRow
          label={zh ? '默认模型' : 'Default model'}
          description={
            zh
              ? '留空则跟随 Chat 默认模型。命令行 --model / --provider 仍优先生效。'
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
            inheritLabel={zh ? '跟随 Chat 模型' : 'Follow Chat model'}
          />
        </SettingRow>

        <SettingRow
          label={zh ? '工具审批策略' : 'Tool approval policy'}
          description={
            zh
              ? '决定 kivio-code 执行工具前是否需要确认。命令行 --no-approve 会强制为"每次确认"。'
              : 'Whether kivio-code confirms before running tools. CLI --no-approve forces "confirm every call".'
          }
        >
          <Select
            value={config.approvalPolicy || 'auto'}
            onChange={(approvalPolicy) => update({ approvalPolicy })}
            options={[
              { value: 'auto', label: zh ? '完全访问' : 'Full access' },
              {
                value: 'readonly_auto_sensitive_confirm',
                label: zh ? '敏感确认' : 'Sensitive confirmation',
              },
              { value: 'always_confirm', label: zh ? '每次确认' : 'Confirm every call' },
            ]}
          />
        </SettingRow>

        <SettingRow
          label={zh ? '读取 CLAUDE.md / .claude 上下文' : 'Read CLAUDE.md / .claude context'}
          description={
            zh
              ? '开启后 kivio-code 会读取项目与全局的 CLAUDE.md / .claude 指令文件(跨工具兼容)。'
              : "When on, kivio-code reads project and global CLAUDE.md / .claude instruction files for cross-tool compatibility."
          }
        >
          <Toggle
            checked={config.readClaudeDir}
            onChange={(readClaudeDir) => update({ readClaudeDir })}
          />
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title={zh ? '全局指令' : 'Global instructions'}>
        <div className="px-1 pb-2 text-[12px] leading-relaxed text-neutral-500 dark:text-neutral-400">
          {zh
            ? '每次运行 kivio-code 都会注入的全局指令。项目根目录的 KIVIO.md / AGENTS.md 会在其后叠加。'
            : "Global instructions injected on every kivio-code run. A project's root KIVIO.md / AGENTS.md layers on top."}
        </div>
        <TextArea
          value={instructions ?? ''}
          onChange={updateInstructions}
          rows={10}
          mono
          placeholder={
            zh
              ? '# 全局指令\n\n例如:始终用中文回复;提交信息遵循 Conventional Commits…'
              : '# Global instructions\n\ne.g. Always answer in English; follow Conventional Commits for messages…'
          }
        />
      </SettingsGroup>

      <SettingsGroup title={zh ? '启动方式' : 'How to launch'}>
        <div className="px-1 py-2 text-[13px] leading-relaxed text-neutral-500 dark:text-neutral-400">
          {zh ? (
            <>
              kivio code 是随应用一起构建的终端编码代理。先点击下方「安装命令行工具」把{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                kivio
              </code>{' '}
              命令加入 PATH,然后在新终端里进入你的项目目录运行{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                kivio code
              </code>{' '}
              即可启动;它会读取上面这些设置(默认模型、审批策略、上下文开关)。加{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                --model provider:model
              </code>{' '}
              可临时覆盖默认模型。
            </>
          ) : (
            <>
              kivio code is the terminal coding agent bundled with this app. First click "Install
              command line tool" below to add the{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                kivio
              </code>{' '}
              command to your PATH, then run{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                kivio code
              </code>{' '}
              in a new terminal from your project directory; it reads the settings above (default
              model, approval policy, context toggle). Pass{' '}
              <code className="rounded bg-neutral-100 px-1.5 py-0.5 font-mono text-[12px] text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                --model provider:model
              </code>{' '}
              to override the default model for a run.
            </>
          )}
        </div>

        <SettingRow
          label={zh ? '安装命令行工具' : 'Install command line tool'}
          description={
            zh
              ? '把 kivio 命令注册进用户 PATH(Windows)/软链到 ~/.local/bin(macOS),装好后在新终端可直接用 kivio code。'
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
                ? '安装中…'
                : 'Installing…'
              : zh
                ? '安装'
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
          {(zh ? '保存失败:' : 'Save failed: ') + error}
        </div>
      )}
    </>
  )
}
