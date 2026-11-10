import type { ComponentType, CSSProperties } from 'react'
// 只引入具体的 Color/Mono 组件文件，不引入品牌 index（会带入 antd），与 ModelIcon 一致。
import Anthropic from '@lobehub/icons/es/Anthropic/components/Mono'
import Apple from '@lobehub/icons/es/Apple/components/Mono'
import Aws from '@lobehub/icons/es/Aws/components/Color'
import Bilibili from '@lobehub/icons/es/Bilibili/components/Color'
import Claude from '@lobehub/icons/es/Claude/components/Color'
import Cloudflare from '@lobehub/icons/es/Cloudflare/components/Color'
import DeepSeek from '@lobehub/icons/es/DeepSeek/components/Color'
import Figma from '@lobehub/icons/es/Figma/components/Color'
import Gemini from '@lobehub/icons/es/Gemini/components/Color'
import Github from '@lobehub/icons/es/Github/components/Mono'
import Google from '@lobehub/icons/es/Google/components/Color'
import HuggingFace from '@lobehub/icons/es/HuggingFace/components/Color'
import Microsoft from '@lobehub/icons/es/Microsoft/components/Color'
import Notion from '@lobehub/icons/es/Notion/components/Mono'
import OpenAI from '@lobehub/icons/es/OpenAI/components/Mono'
import Vercel from '@lobehub/icons/es/Vercel/components/Mono'

type SiteGlyph = ComponentType<{ size?: string | number; style?: CSSProperties; className?: string; 'aria-hidden'?: boolean | 'true' | 'false' }>

/**
 * 常见站点的内置品牌图标。只用打包内的图标，不请求远程 favicon：
 * 渲染正文不应触发网络请求，也不泄露用户正在看的链接。
 * 顺序有意义：更具体的子域名（gemini.google.com）排在主域名前。
 */
const SITE_ICONS: ReadonlyArray<readonly [string, SiteGlyph]> = [
  ['github.com', Github],
  ['githubusercontent.com', Github],
  ['github.io', Github],
  ['huggingface.co', HuggingFace],
  ['hf.co', HuggingFace],
  ['claude.ai', Claude],
  ['claude.com', Claude],
  ['anthropic.com', Anthropic],
  ['openai.com', OpenAI],
  ['chatgpt.com', OpenAI],
  ['gemini.google.com', Gemini],
  ['ai.google.dev', Gemini],
  ['google.com', Google],
  ['microsoft.com', Microsoft],
  ['apple.com', Apple],
  ['notion.so', Notion],
  ['notion.site', Notion],
  ['vercel.com', Vercel],
  ['vercel.app', Vercel],
  ['figma.com', Figma],
  ['cloudflare.com', Cloudflare],
  ['aws.amazon.com', Aws],
  ['deepseek.com', DeepSeek],
  ['bilibili.com', Bilibili],
  ['b23.tv', Bilibili],
]

/** 返回网页链接对应的站点图标；未收录的站点返回 undefined（保持纯文字链接）。 */
function siteIconFor(url: string): SiteGlyph | undefined {
  let host: string
  try {
    host = new URL(url).hostname.toLowerCase()
  } catch {
    return undefined
  }
  if (!host) return undefined
  return SITE_ICONS.find(([domain]) => host === domain || host.endsWith(`.${domain}`))?.[1]
}

export function SiteIcon({ url }: { url: string }) {
  const Icon = siteIconFor(url)
  if (!Icon) return null
  // U+2060 禁止图标与后面的文字之间换行。
  return <><Icon aria-hidden size="1em" className="mr-1 inline-block align-[-0.125em]" />{'\u2060'}</>
}
