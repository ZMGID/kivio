import { invoke } from '@tauri-apps/api/core'
import { useT } from '../components/i18n'
import { useState } from 'react'
import { artifactDataUrl, isFileArtifact } from './artifacts'
import { FileChip } from './fileChip'
import { fileLocationAction } from './fileLocation'
import type { ChatToolArtifact } from './types'

function artifactPath(artifact: ChatToolArtifact): string {
  return artifact.path ?? artifact.filePath ?? artifact.localPath ?? ''
}

async function openGeneratedArtifact(artifact: ChatToolArtifact) {
  const path = artifactPath(artifact)
  if (path) {
    await invoke('chat_open_generated_artifact', { path })
    return
  }

  const dataUrl = artifactDataUrl(artifact)
  if (!dataUrl) throw new Error('Artifact has no available file content')
  // 没有 path 的旧 artifact：落成临时文件再交给系统默认程序。
  // 不能用 `window.open(dataUrl)` —— 那条在 Tauri 里由 webview 自己处理，不会打开默认程序。
  await invoke('open_data_url_file', { name: artifact.name ?? 'file.bin', dataUrl })
}

export function ArtifactFileChip({
  artifact,
  conversationId,
  variant = 'card',
}: {
  artifact: ChatToolArtifact
  conversationId?: string | null
  variant?: 'card' | 'inline'
}) {
  const t = useT()
  const [error, setError] = useState(false)
  return <span className="not-prose" data-artifact-id={artifact.id ?? undefined}>
    <FileChip variant={variant} name={artifact.name} onRevealLocation={fileLocationAction(artifactPath(artifact), conversationId)} ariaLabel={`${t.chatMsgOpenFile} ${artifact.name}`} onClick={() => {
      setError(false)
      void openGeneratedArtifact(artifact).catch(() => setError(true))
    }} />
    {error && <span role="status" className="ml-1 text-xs text-neutral-500">{t.chatMsgFileOpenFailed}</span>}
  </span>
}

export function GeneratedFileArtifacts({ artifacts, includeImages = false, conversationId }: { artifacts: ChatToolArtifact[]; includeImages?: boolean; conversationId?: string | null }) {
  const fileArtifacts = includeImages ? artifacts : artifacts.filter(isFileArtifact)
  if (fileArtifacts.length === 0) return null

  return (
    <div className="not-prose mt-3 flex min-w-0 max-w-full flex-wrap gap-2">
      {fileArtifacts.map((artifact, index) => (
        <ArtifactFileChip
          key={`${artifact.path || artifact.name}-${index}`}
          artifact={artifact}
          conversationId={conversationId}
        />
      ))}
    </div>
  )
}
