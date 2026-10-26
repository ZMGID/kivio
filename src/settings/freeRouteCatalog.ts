/** Display metadata for FreeRoute vendors. Routing/catalog authority lives in Rust. */
export type FreeRouteVendorMeta = {
  id: 'opencode' | 'openrouter' | 'sensenova' | 'groq'
  name: string
  signupUrl: string
  allowsAnonymous: boolean
}

export const FREEROUTE_VENDOR_META: FreeRouteVendorMeta[] = [
  {
    id: 'opencode',
    name: 'OpenCode Zen',
    signupUrl: 'https://opencode.ai/zen',
    allowsAnonymous: true,
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    signupUrl: 'https://openrouter.ai/keys',
    allowsAnonymous: false,
  },
  {
    id: 'sensenova',
    name: 'SenseNova',
    signupUrl: 'https://platform.sensenova.ai',
    allowsAnonymous: false,
  },
  {
    id: 'groq',
    name: 'Groq',
    signupUrl: 'https://console.groq.com/keys',
    allowsAnonymous: false,
  },
]
