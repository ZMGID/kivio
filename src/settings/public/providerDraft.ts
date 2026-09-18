import type { ProviderOAuthConfig, ProviderRequestConfig } from '../../api/tauri'

/** Explicit defaults for a brand-new unsaved provider draft; persisted values come from Rust. */
export function createProviderRequestDraft(
  oauth?: ProviderOAuthConfig['provider'],
): ProviderRequestConfig {
  return {
    ...(oauth ? { oauth: { provider: oauth } } : {}),
    customHeaders: [],
    useSystemProxy: true,
    promptCacheRetention: 'short',
    cliIdentity: '',
    cliIdentityVersion: '',
  }
}
