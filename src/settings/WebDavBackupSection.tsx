import { useEffect, useState } from 'react'
import './backup.css'
import { ArrowLeft, Cloud, Download, RefreshCw, Server, ShieldCheck, Trash2, Upload } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import type { Lang } from '../components/i18n'
import { Input, Select, Toggle } from './public/controls'
import { BackupDialog } from './BackupDialog'
import type { BackupKind, useSettingsBackupController } from './useSettingsBackupController'

export function WebDavBackupSection({ controller, lang }: {
  controller: ReturnType<typeof useSettingsBackupController>
  lang: Lang
}) {
  const zh = lang === 'zh'
  const config = controller.webdavConfig
  const [draft, setDraft] = useState<{ url: string; username: string; directory: string; password: string | null } | null>(null)
  const [editing, setEditing] = useState(false)
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null)
  const [choosing, setChoosing] = useState(false)
  const [browsing, setBrowsing] = useState(false)
  const [kind, setKind] = useState<BackupKind>('configuration')
  const [trusted, setTrusted] = useState(false)
  const disabled = controller.busy || controller.restartRequired
  const values = draft ?? (config ? { url: config.url, username: config.username, directory: config.directory, password: null } : null)
  const providers = [
    { id: 'jianguoyun', name: zh ? '坚果云' : 'Jianguoyun', hint: 'dav.jianguoyun.com', url: 'https://dav.jianguoyun.com/dav/', placeholder: 'https://dav.jianguoyun.com/dav/', help: zh ? '账号填写坚果云注册邮箱，密码使用「安全选项 → 第三方应用管理」生成的应用密码。默认单文件上限 500M，单次列表最多 750 项；建议使用独立备份目录。' : 'Use your registered email and an app password from Security → Third-party applications. Default limit: 500M per file and 750 entries per listing. Use a dedicated backup folder.' },
    { id: 'nextcloud', name: 'Nextcloud', hint: zh ? '自建云存储' : 'Self-hosted storage', url: '', placeholder: 'https://cloud.example.com/remote.php/dav/files/USERNAME/', help: zh ? '从 Nextcloud「文件 → 文件设置」复制个人 WebDAV 地址，使用安全设置中生成的应用密码；不要填写网页版或公开分享链接。' : 'Copy your personal WebDAV URL from Files → Files settings and use an app password. Do not use the web portal or a public share link.' },
    { id: 'synology', name: zh ? '群晖 NAS' : 'Synology NAS', hint: 'WebDAV Server', url: '', placeholder: 'https://nas.example.com:5006/SHARED_FOLDER/', help: zh ? '先启用 WebDAV Server；HTTPS 默认端口 5006。地址包含已有共享文件夹，账号需有读写权限，证书需可信且匹配域名。' : 'Enable WebDAV Server (HTTPS default: 5006). Include an existing shared folder in the URL. Grant read/write access and use a trusted certificate matching the hostname.' },
    { id: 'custom', name: zh ? '自定义 WebDAV' : 'Custom WebDAV', hint: zh ? '其他兼容 WebDAV 的服务' : 'Any compatible WebDAV service', url: '', placeholder: 'https://dav.example.com/', help: zh ? '填写服务商提供的 WebDAV 地址和凭据。测试连接会验证目录创建、上传、重命名、回读、列表和删除权限。' : 'Use the WebDAV endpoint and credentials from your provider. The connection test checks folder creation, upload, rename, read-back, listing and deletion.' },
  ]
  let detected = 'custom'
  try {
    const url = new URL(config?.url || '')
    if (url.hostname === 'dav.jianguoyun.com') detected = 'jianguoyun'
    else if (url.pathname.includes('/remote.php/dav/files/')) detected = 'nextcloud'
    else if (url.port === '5005' || url.port === '5006') detected = 'synology'
  } catch { /* An unconfigured connection starts with custom WebDAV. */ }
  const provider = providers.find((item) => item.id === (selectedProvider ?? detected)) ?? providers[3]
  const dirty = values && config && (values.url !== config.url || values.username !== config.username || values.directory !== config.directory || values.password !== null)
  const ready = config?.url && !dirty
  const edit = (key: 'url' | 'username' | 'directory' | 'password', value: string) => {
    if (values) setDraft({ ...values, [key]: value })
  }
  const run = controller.runWebDav
  useEffect(() => { void run({ type: 'load' }) }, [run])
  const closeEditor = () => { setEditing(false); setChoosing(false) }
  const openEditor = () => { setChoosing(!config?.url); setEditing(true); if (!config) void run({ type: 'load' }) }
  const status = controller.statusSource === 'webdav' && (controller.busy || controller.status) ? <p role="status" aria-live="polite" className="backup-status whitespace-pre-wrap [overflow-wrap:anywhere]" style={{ color: controller.status?.kind === 'err' ? 'var(--danger)' : 'var(--text-muted)' }}>{controller.busy ? controller.webdavPhase || (zh ? '正在处理…' : 'Processing…') : controller.status?.msg}</p> : null
  return <section className="backup-section">
    <h3 className="backup-section-title"><Cloud size={16} />{zh ? '云端备份' : 'Cloud backup'}<span className="backup-tag">WebDAV</span></h3>
    <div className="backup-surface">
      <div className="backup-row">
        <div className="backup-service-icon"><Cloud size={22} /></div>
        <div className="backup-copy"><p className="kv-row-label">{config?.url ? (zh ? '已配置 WebDAV' : 'WebDAV configured') : (zh ? '连接你的云端存储' : 'Connect your cloud storage')}</p>
          <p className="kv-row-desc backup-ellipsis" title={config?.url}>{config?.url || (zh ? '通过 WebDAV，在多台电脑间手动备份与恢复。' : 'Back up and restore between computers using WebDAV.')}</p>
          {config?.url && <p className="kv-row-desc">{zh ? '手动备份' : 'Manual backup'} · {config.directory || '/'}</p>}
        </div>
        <Button size="sm" variant={config?.url ? 'default' : 'primary'} disabled={disabled} onClick={openEditor}>{config?.url ? (zh ? '管理连接' : 'Manage connection') : (zh ? '配置 WebDAV' : 'Configure WebDAV')}</Button>
      </div>
      {config?.url && <div className="backup-row backup-divider">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '远程备份' : 'Remote backups'}</p><p className="kv-row-desc">{zh ? '上传新版本，或选择已有备份恢复。' : 'Upload a new version or restore an existing backup.'}</p></div>
        <div className="backup-actions">
          <Select ariaLabel={zh ? '远程备份类型' : 'Remote backup type'} value={kind} onChange={(value) => setKind(value as BackupKind)} options={[{ value: 'configuration', label: zh ? '配置备份' : 'Configuration' }, { value: 'full', label: zh ? '完全备份' : 'Full backup' }]} disabled={disabled} />
          <Button size="sm" disabled={disabled || !ready} onClick={() => void run({ type: 'upload', kind })}><Upload size={13} />{zh ? '上传备份' : 'Upload backup'}</Button>
          <Button size="sm" disabled={disabled || !ready} onClick={() => { setBrowsing(true); void run({ type: 'list' }) }}><Download size={13} />{zh ? '恢复备份' : 'Restore backup'}</Button>
        </div>
      </div>}
      {dirty && <p className="kv-row-desc">{zh ? '连接有未保存的更改，请先保存或放弃更改。' : 'Save or discard connection changes before transferring backups.'}</p>}
    </div>
    {!editing && !browsing && status}
    {editing && <BackupDialog title={choosing ? (zh ? '选择 WebDAV 服务' : 'Choose a WebDAV service') : `${provider.name} ${zh ? '连接设置' : 'connection settings'}`} description={zh ? '连接信息仅保存在当前设备。' : 'Connection details are stored on this device only.'} closeLabel={zh ? '关闭' : 'Close'} locked={disabled} onClose={closeEditor}>
      {choosing ? <div className="backup-providers">
        {providers.map((item) => <div key={item.id} className="backup-provider-row"><Server size={18} /><div className="backup-copy"><p className="kv-row-label">{item.name}</p><p className="kv-row-desc">{item.hint}</p></div><Button size="sm" disabled={disabled || !values} onClick={() => {
          if (item.id !== provider.id && values) setDraft({ url: item.url, username: '', password: '', directory: 'Kivio' })
          else if (item.url && !values?.url) edit('url', item.url)
          setSelectedProvider(item.id); setChoosing(false)
        }}>{zh ? `选择${item.name}` : `Choose ${item.name}`}</Button></div>)}
      </div> : <>
        <Button size="sm" variant="ghost" disabled={disabled} onClick={() => setChoosing(true)}><ArrowLeft size={13} />{zh ? '选择服务' : 'Choose service'}</Button>
        <p className="kv-row-desc mt-3">{provider.help}</p>
        {values && <div className="backup-connection">
      <div className="backup-fields">
        <label className="min-w-0"><span className="kv-row-desc">{zh ? '服务器地址' : 'Server URL'}</span><Input aria-label={zh ? 'WebDAV 服务器地址' : 'WebDAV server URL'} value={values.url} onChange={(value) => edit('url', value)} placeholder={provider.placeholder} disabled={disabled} /></label>
        <label className="min-w-0"><span className="kv-row-desc">{zh ? '账号' : 'Username'}</span><Input aria-label={zh ? 'WebDAV 账号' : 'WebDAV username'} value={values.username} onChange={(value) => edit('username', value)} autoComplete="off" disabled={disabled} /></label>
        <label className="min-w-0"><span className="kv-row-desc">{zh ? '密码 / 应用密码' : 'Password / app password'}</span><Input aria-label={zh ? 'WebDAV 密码' : 'WebDAV password'} type="password" value={values.password ?? ''} onChange={(value) => edit('password', value)} placeholder={config?.hasPassword && values.password === null ? (zh ? '已保存，未修改则保留' : 'Saved; leave unchanged to keep') : ''} autoComplete="new-password" disabled={disabled} /></label>
        <label className="min-w-0"><span className="kv-row-desc">{zh ? '远程目录' : 'Remote directory'}</span><Input aria-label={zh ? 'WebDAV 远程目录' : 'WebDAV remote directory'} value={values.directory} onChange={(value) => edit('directory', value)} placeholder="Kivio" disabled={disabled} /></label>
      </div>
      <div className="flex flex-wrap items-center gap-2 py-3">
        <Button size="sm" disabled={disabled} onClick={() => void run({ type: 'test', input: values })}>{zh ? '测试连接' : 'Test connection'}</Button>
        <Button size="sm" variant="primary" disabled={disabled || !dirty && Boolean(config?.url)} onClick={async () => { if (await run({ type: 'save', input: values })) setDraft(null) }}>{zh ? '保存配置' : 'Save configuration'}</Button>
        {dirty && <span className="kv-row-desc">{zh ? '保存后可上传和读取备份。' : 'Save changes to upload or browse backups.'}</span>}
      </div>
        </div>}
        <div className="backup-note"><ShieldCheck size={16} /><span>{zh ? '备份包含未加密的密钥，请使用可信的 HTTPS 服务。' : 'Backups contain unencrypted secrets. Use a trusted HTTPS service.'}</span></div>
        {dirty && <Button size="sm" variant="ghost" disabled={disabled} onClick={() => { setDraft(null); setSelectedProvider(null) }}>{zh ? '放弃更改' : 'Discard changes'}</Button>}
      </>}
      {status}
    </BackupDialog>}
    {browsing && <BackupDialog title={zh ? '恢复云端备份' : 'Restore a cloud backup'} description={zh ? '恢复将替换对应数据；完全恢复需重启。' : 'Restore replaces the corresponding data. Full restore requires a restart.'} closeLabel={zh ? '关闭' : 'Close'} locked={controller.busy} onClose={() => setBrowsing(false)}>
      <div className="backup-row backup-divider">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '从云端恢复' : 'Restore from the cloud'}</p><p className="kv-row-desc">{zh ? '选择一个备份恢复到当前电脑。' : 'Choose a backup to restore on this computer.'}</p></div>
        <Button size="sm" variant="ghost" disabled={disabled || !ready} onClick={() => void run({ type: 'list' })}><RefreshCw size={13} />{zh ? '刷新备份列表' : 'Refresh backups'}</Button>
      </div>
      <div className="backup-trust">
        <Toggle ariaLabel={zh ? '信任远程备份' : 'Trust remote backup'} checked={trusted} onChange={setTrusted} disabled={disabled} />
        <span className="kv-row-desc">{zh ? '信任备份，恢复时保留执行权限与任务状态' : 'Trust backup and retain execution permissions and task states'}</span>
      </div>
      {controller.webdavFiles === null && <p className="kv-row-desc backup-empty">{zh ? '刷新列表，查看此目录中的备份。' : 'Refresh to view backups in this directory.'}</p>}
      {controller.webdavFiles !== null && <div className="custom-scrollbar max-h-72 overflow-auto">
        {controller.webdavFiles.length === 0 ? <p className="kv-row-desc backup-empty">{zh ? '此目录暂无 Kivio 备份。' : 'No Kivio backups in this directory.'}</p> : controller.webdavFiles.map((backup) => <div key={backup.name} className="backup-file">
          <div className="backup-copy">
            <p className="kv-row-label">{backup.kind === 'full' ? (zh ? '完全备份' : 'Full backup') : (zh ? '配置备份' : 'Configuration')}</p>
            <p className="kv-row-desc backup-file-meta">{backup.modified && !Number.isNaN(Date.parse(backup.modified)) ? new Date(backup.modified).toLocaleString(zh ? 'zh-CN' : 'en-US', { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : (zh ? '时间未知' : 'Unknown date')} · {backup.size < 1024 * 1024 ? `${(backup.size / 1024).toFixed(1)} KB` : `${(backup.size / 1024 / 1024).toFixed(1)} MB`}</p>
            <p className="kv-row-desc backup-ellipsis" title={backup.name}>{backup.name}</p>
          </div>
          <div className="backup-actions">
          <Button size="sm" aria-label={zh ? `恢复 ${backup.name}` : `Restore ${backup.name}`} disabled={disabled || !ready} onClick={async () => { if (await run({ type: 'restore', backup, trusted })) setBrowsing(false) }}><Download size={13} />{zh ? '恢复' : 'Restore'}</Button>
          <IconButton variant="ghost" label={zh ? `删除 ${backup.name}` : `Delete ${backup.name}`} disabled={disabled || !ready} onClick={() => void run({ type: 'delete', name: backup.name })}><Trash2 size={14} /></IconButton>
          </div>
        </div>)}
      </div>}
      {status}
    </BackupDialog>}
  </section>
}
