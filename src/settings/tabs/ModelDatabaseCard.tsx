import { useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { api, type ModelDatabaseLocalInfo, type ModelDatabaseRemoteInfo } from '../../api/tauri'
import type { Lang } from '../../components/i18n'
import { Input, SettingsGroup, SettingRow } from '../components'

const URL_STORAGE_KEY = 'kivio_model_db_url'
/** 作者在 Gitee 维护的模型库 raw 地址；用户可换成任意 HTTPS 直链（GitHub raw / jsdelivr / 自建）。 */
const DEFAULT_MODEL_DB_URL = 'https://gitee.com/zmgid/kivio-model-database/raw/main/modelDatabase.json'

/**
 * 模型库远程更新卡片：显示当前生效版本，从远程地址一键拉取最新的 modelDatabase.json。
 * 覆盖文件写入应用数据目录，重启应用后由内置的合并机制生效（见 chat/model_metadata.rs）。
 */
export function ModelDatabaseCard({ lang }: { lang: Lang }) {
  const zh = lang === 'zh'
  const [local, setLocal] = useState<ModelDatabaseLocalInfo | null>(null)
  const [url, setUrl] = useState(() => localStorage.getItem(URL_STORAGE_KEY) || DEFAULT_MODEL_DB_URL)
  const [remote, setRemote] = useState<ModelDatabaseRemoteInfo | null>(null)
  const [busy, setBusy] = useState<'check' | 'install' | null>(null)
  const [message, setMessage] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    let alive = true
    api.modelDatabaseLocalInfo().then((info) => {
      if (alive) setLocal(info)
    }).catch(() => { /* 旧版本无此命令时静默 */ })
    return () => { alive = false }
  }, [])

  const check = async () => {
    setBusy('check'); setError(''); setMessage(''); setRemote(null)
    try {
      const info = await api.modelDatabaseRemoteCheck(url.trim())
      setRemote(info)
      const effective = local?.effectiveVersion
      setMessage(effective === info.version
        ? (zh ? '远程与当前版本一致，已是最新。' : 'Remote matches the current version — up to date.')
        : (zh ? `远程有新版本 ${info.version}（${info.entries} 个模型）。` : `Remote has a newer version ${info.version} (${info.entries} models).`))
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(null)
    }
  }

  const install = async () => {
    setBusy('install'); setError(''); setMessage('')
    try {
      localStorage.setItem(URL_STORAGE_KEY, url.trim())
      const info = await api.modelDatabaseRemoteInstall(url.trim())
      setRemote(info)
      const refreshed = await api.modelDatabaseLocalInfo()
      setLocal(refreshed)
      setMessage(zh
        ? `已安装模型库 ${info.version}（${info.entries} 个模型）。重启 Kivio 后生效。`
        : `Installed model database ${info.version} (${info.entries} models). Restart Kivio to apply.`)
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <SettingsGroup title={zh ? '模型库' : 'Model database'}>
      <SettingRow
        label={zh ? '当前版本' : 'Current version'}
        description={zh
          ? (local
            ? (local.userVersion
              ? `生效 ${local.effectiveVersion}（远程更新覆盖，内置 ${local.builtinVersion}）。`
              : `生效 ${local.effectiveVersion}（内置）。`)
            : '')
          : (local
            ? (local.userVersion
              ? `Effective ${local.effectiveVersion} (remote overlay, built-in ${local.builtinVersion}).`
              : `Effective ${local.effectiveVersion} (built-in).`)
            : '')}
      >
        <span className="kv-tag ok">{local?.effectiveVersion ?? '…'}</span>
      </SettingRow>
      <SettingRow
        label={zh ? '模型库地址' : 'Database URL'}
        description={zh
          ? '指向 modelDatabase.json 的 HTTPS 直链（Gitee / GitHub raw / 自建镜像均可）。更新写入本机后重启生效。'
          : 'An HTTPS link to modelDatabase.json (Gitee / GitHub raw / your own mirror). Saved locally; restart to apply.'}
        stack
      >
        <Input value={url} onChange={setUrl} placeholder={DEFAULT_MODEL_DB_URL} mono />
      </SettingRow>
      <div className="flex items-center gap-2 px-1 pb-2">
        <button
          type="button"
          onClick={check}
          disabled={busy !== null || !url.trim()}
          className="flex items-center gap-1.5 rounded-md border border-neutral-300 px-3 py-1.5 text-sm disabled:opacity-50 dark:border-neutral-700"
        >
          <RefreshCw size={13} className={busy === 'check' ? 'animate-spin' : ''} />
          {zh ? '检查更新' : 'Check updates'}
        </button>
        <button
          type="button"
          onClick={install}
          disabled={busy !== null || !url.trim()}
          className="rounded-md bg-neutral-900 px-3 py-1.5 text-sm text-white disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900"
        >
          {busy === 'install'
            ? (zh ? '安装中…' : 'Installing…')
            : remote && local && remote.version !== local.effectiveVersion
              ? (zh ? `更新到 ${remote.version}` : `Update to ${remote.version}`)
              : (zh ? '下载安装' : 'Download & install')}
        </button>
      </div>
      {message && !error && <p className="kv-row-desc px-1 text-emerald-600 dark:text-emerald-400">{message}</p>}
      {error && <p className="kv-row-desc px-1 text-red-500">{error}</p>}
    </SettingsGroup>
  )
}
