import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type ConnectorOAuthArgs, type OAuthDevicePrompt } from '../api/tauri'

export function isBuiltinGithubOAuth(url: string | undefined): boolean {
  try {
    const parsed = new URL(url ?? '')
    return parsed.protocol === 'https:' && parsed.hostname === 'api.githubcopilot.com'
      && !parsed.port && !parsed.username && !parsed.password && !parsed.search && !parsed.hash
      && ['/mcp', '/mcp/'].includes(parsed.pathname)
  } catch { return false }
}

/** Shared authorization lifetime for the MCP page and connector directory. */
export function useConnectorOAuth() {
  const active = useRef<AbortController | null>(null)
  const [prompt, setPrompt] = useState<OAuthDevicePrompt | null>(null)
  const cancel = useCallback(() => {
    active.current?.abort()
    setPrompt(null)
  }, [])
  useEffect(() => () => { active.current?.abort() }, [])
  const connect = useCallback(async (args: ConnectorOAuthArgs) => {
    // A second click while authorization is running cannot start another flow.
    if (active.current) return undefined
    const controller = new AbortController()
    active.current = controller
    try {
      const server = await api.connectorOauthConnect(args, (value) => {
        if (!controller.signal.aborted) setPrompt(value)
      }, controller.signal)
      return controller.signal.aborted ? undefined : server
    } catch (error) {
      if (controller.signal.aborted || String(error).includes('OAUTH_CANCELLED')) return undefined
      throw error
    } finally {
      if (active.current === controller) {
        active.current = null
        if (!controller.signal.aborted) setPrompt(null)
      }
    }
  }, [])
  return { connect, prompt, cancel }
}
