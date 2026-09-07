import { describe, expect, it } from 'vitest'
import { outlineItemsForSource, parseMarkdownHeadingOutline, primaryHeadingDepth } from './markdownHeadingOutline'

describe('parseMarkdownHeadingOutline', () => {
  it('keeps root H1–H3 including Setext and semantic inline text', () => {
    expect(parseMarkdownHeadingOutline(`
# Title **bold** [link](https://example.com)

Second title
------------

### \`Third\` 中文

#### ignored
`)).toEqual([
      { depth: 1, title: 'Title bold link', ordinal: 0 },
      { depth: 2, title: 'Second title', ordinal: 1 },
      { depth: 3, title: 'Third 中文', ordinal: 2 },
    ])
  })

  it('excludes code fences, quoted headings, details HTML and H4–H6', () => {
    expect(parseMarkdownHeadingOutline(`
\`\`\`
# source code
\`\`\`

> ## Quoted

<details>
## Hidden
</details>

### Kept
##### ignored
`)).toEqual([{ depth: 3, title: 'Kept', ordinal: 0 }])
  })

  it('uses the shallowest actual heading as the primary level and stable duplicate anchors', () => {
    const items = outlineItemsForSource('message-1', '## Repeat\n### Child\n## Repeat')
    expect(primaryHeadingDepth(items)).toBe(2)
    expect(items.map((item) => item.anchorId)).toEqual([
      'user-content-chat-heading-message-1-0',
      'user-content-chat-heading-message-1-1',
      'user-content-chat-heading-message-1-2',
    ])
  })
})
