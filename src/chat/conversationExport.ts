// eslint-disable-next-line no-control-regex -- control chars are intentionally stripped from filenames
const INVALID_FILENAME_CHARS = /[<>:"/\\|?*\u0000-\u001f]/g
const WINDOWS_RESERVED_NAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?$/i
const MAX_FILENAME_STEM_CHARS = 80

export function conversationMarkdownFilename(title: string): string {
  let stem = title
    .replace(INVALID_FILENAME_CHARS, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/[. ]+$/g, '')

  stem = [...stem].slice(0, MAX_FILENAME_STEM_CHARS).join('').replace(/[. ]+$/g, '')
  if (!stem) stem = 'conversation'
  if (WINDOWS_RESERVED_NAME.test(stem)) stem = `conversation-${stem}`
  return `${stem}.md`
}
