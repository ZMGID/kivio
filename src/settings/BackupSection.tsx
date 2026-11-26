import { ErrorNotice } from '../components/AppNotice'
import { useState } from 'react'
import './backup.css'
import { WebDavBackupSection } from './WebDavBackupSection'
import { Download, Upload, ShieldCheck } from 'lucide-react'
import { Button } from '../components/Button'
import { Toggle } from './public/controls'
import type { Lang } from '../components/i18n'
import type { useSettingsBackupController } from './useSettingsBackupController'

export function BackupSection({ controller, lang }: {
  controller: ReturnType<typeof useSettingsBackupController>
  lang: Lang
}) {
  const [trusted, setTrusted] = useState(false)
  const zh = lang === 'zh'
  const disabled = controller.busy
  return <div className="backup-page">
    <section className="backup-section"><h3 className="backup-section-title">{zh ? '配置备份' : 'Configuration backup'}</h3><div className="backup-surface">
      <div className="backup-row">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '导出到文件' : 'Export to a file'}</p><p className="kv-row-desc">{zh ? '设置、供应商、模型、助手和集。' : 'Settings, providers, models, assistants and sets.'}</p></div>
        <Button size="sm" disabled={disabled} onClick={controller.exportBackup}><Upload size={13} />{zh ? '导出配置' : 'Export configuration'}</Button>
      </div>
      <div className="backup-divider">
        <div className="backup-row">
          <div className="backup-copy"><p className="kv-row-label">{zh ? '从文件恢复' : 'Restore from a file'}</p><p className="kv-row-desc">{zh ? '选择 ZIP 配置备份（兼容旧 JSON），恢复设置、供应商、模型、助手和集。' : 'Choose a ZIP configuration backup (legacy JSON supported) to restore settings, providers, models, assistants and sets.'}</p></div>
          <Button size="sm" disabled={disabled} onClick={trusted ? controller.restoreBackup : controller.importBackup}><Download size={13} />{zh ? '导入备份' : 'Import backup'}</Button>
        </div>
        <div className="backup-trust">
          <Toggle ariaLabel={zh ? '信任此备份' : 'Trust this backup'} checked={trusted} onChange={setTrusted} disabled={disabled} />
          <span className="kv-row-desc">{zh ? '信任此备份，恢复时保留执行权限' : 'Trust this backup and retain execution permissions'}</span>
        </div>
      </div>
    </div></section>
    <section className="backup-section">
      <h3 className="backup-section-title">{zh ? '聊天全量备份' : 'Full chat backup'}</h3>
      <div className="backup-surface"><div className="backup-row">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '项目、对话、附件、笔记和记忆' : 'Projects, conversations, attachments, notes and memory'}</p>
          <p className="kv-row-desc">{zh ? '包含助手、集、项目记录、全部对话、分支、草稿、附件、笔记、文件夹、笔记回收站，以及 L1 在线记忆和 L2 长期记忆。恢复会替换备份包含的数据并重启应用。' : 'Includes assistants, sets, project records, conversations, branches, drafts, attachments, notes, note folders and Trash, plus L1 online memory and L2 long-term memory. Restore replaces the data covered by the backup and restarts the app.'}</p>
          <p className="kv-row-desc">{zh ? '不包含项目代码、外部链接文件或 CLI 原生会话。换电脑后可能需要重新指定项目目录，外部 CLI 不保证能续聊。' : 'Excludes project source files, externally linked files and CLI native sessions. Project paths may need updating on another computer; CLI session continuation is not guaranteed.'}</p>
        </div>
        <div className="backup-actions">
          <Button size="sm" disabled={disabled} onClick={controller.exportChatBackup}><Upload size={13} />{zh ? '导出全量备份' : 'Export full backup'}</Button>
          <Button size="sm" disabled={disabled} onClick={controller.restoreChatBackup}><Download size={13} />{zh ? '恢复全量备份' : 'Restore full backup'}</Button>
        </div>
      </div></div>
    </section>
    <WebDavBackupSection controller={controller} lang={lang} />
    <div className="backup-note"><ShieldCheck size={16} /><span>{zh ? '备份可能含密钥或私人聊天，请妥善保管。配置备份不含聊天；聊天全量备份不含项目代码。' : 'Backups may contain secrets or private chats. Keep them safe. Configuration backups exclude chats; chat backups exclude project source files.'}</span></div>
    <ErrorNotice message={controller.statusSource === 'local' && !controller.busy && controller.status?.kind === 'err' ? controller.status.msg : null} />
    {controller.statusSource === 'local' && (controller.busy || (controller.status && controller.status.kind !== 'err')) && <p role="status" aria-live="polite" className="text-[12px] whitespace-pre-wrap [overflow-wrap:anywhere]" style={{ color: controller.status?.kind === 'err' ? 'var(--danger)' : 'var(--text-muted)' }}>
      {controller.busy ? (zh ? '正在处理备份，请稍候…' : 'Processing backup…') : controller.status?.msg}
    </p>}
  </div>
}
